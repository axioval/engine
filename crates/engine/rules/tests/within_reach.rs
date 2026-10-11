//! `sum_within`, `min_within`, `max_within` and `mean_within` over the
//! counterparts within a walking distance.
//!
//! The stubs answer only what a test declares and panic on any other
//! distance or route, which also proves the plan broad phase keeps the
//! counterparts surely beyond the reach from being walked to.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    BlockedMetricRouteEvidence, Bounds3, CentrePlacement, CompleteMetricEvidence,
    ElevationInterval, GeometryFidelity, LengthInterval, MetricRouteEvidence, MetricRouteOutcome,
    MetricRouteRequest, MetricRoutingError, MetricRoutingService, MetricRoutingServiceHandle,
    ObjectBounds, PlanCentre, PlanLength, PlanSpan, PlanSpanError, PlanSpanService,
    PlanSpanServiceHandle, ProjectedDistanceEvidence, ProximityError, ProximityEvidence,
    ProximityProjection, ProximityRequest, ProximityService, ProximityServiceHandle,
    ServiceRegistry, VerticalExtent, VerticalExtentError, VerticalExtentService,
    VerticalExtentServiceHandle,
};
use axioval_ir::{Evidence, ObjectId, PropertyValue};
use common::{Model, id, kind, rule, source};

/// How a stubbed walk from the room to a counterpart ends.
#[derive(Clone, Copy, Debug)]
enum Walk {
    Reachable(f64, f64),
    Blocked,
    Refused,
}

/// Plan distances and walks from the room, per counterpart.
#[derive(Default)]
struct Floor {
    /// Counterpart -> plan distance between the bodies.
    plan: BTreeMap<String, (f64, f64)>,
    /// Counterpart -> how the walk from the room ends.
    walks: BTreeMap<String, Walk>,
}

impl Floor {
    fn counterpart(mut self, local: &str, plan: (f64, f64), walk: Walk) -> Self {
        self.plan.insert(local.into(), plan);
        self.walks.insert(local.into(), walk);
        self
    }

    fn register(self, services: &mut ServiceRegistry) {
        let shared = Arc::new(self);
        services
            .register(ProximityServiceHandle::new(shared.clone()))
            .unwrap();
        services
            .register(PlanSpanServiceHandle::new(shared.clone()))
            .unwrap();
        services
            .register(VerticalExtentServiceHandle::new(shared.clone()))
            .unwrap();
        services
            .register(MetricRoutingServiceHandle::new(shared))
            .unwrap();
    }
}

impl ProximityService for Floor {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        // Every body overlaps every other: the broad phase proposes each
        // pair, and the measured plan distance decides which are walked.
        ObjectBounds::try_new(
            object.clone(),
            Bounds3::try_new([0.0; 3], [1.0; 3])?,
            GeometryFidelity::Exact,
        )
    }

    fn measure_proximity(&self, _: &ProximityRequest) -> Result<ProximityEvidence, ProximityError> {
        panic!("distances are measured through measure_distance")
    }

    fn measure_distance(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProjectedDistanceEvidence, ProximityError> {
        assert!(
            matches!(request.projection(), ProximityProjection::Horizontal),
            "a walk is bounded by the plan distance"
        );
        let local = &request.counterpart().local_id;
        let (lower, upper) = self.plan[local];
        ProjectedDistanceEvidence::try_new(
            request.clone(),
            lower,
            upper,
            GeometryFidelity::Exact,
            Evidence::exact(source(), format!("plan:{local}")),
        )
    }
}

impl PlanSpanService for Floor {
    fn measure_diameter(&self, object: &ObjectId) -> Result<PlanLength, PlanSpanError> {
        panic!("unexpected diameter of {object}")
    }

    fn measure_span(
        &self,
        first: &ObjectId,
        _: &ObjectId,
        _: PlanSpan,
    ) -> Result<PlanLength, PlanSpanError> {
        panic!("unexpected span from {first}")
    }

    fn measure_centre(&self, object: &ObjectId) -> Result<PlanCentre, PlanSpanError> {
        PlanCentre::try_new(
            object.clone(),
            [1.0, 2.0],
            0.0,
            CentrePlacement::Inside,
            Evidence::exact(source(), format!("centre:{}", object.local_id)),
        )
    }
}

impl VerticalExtentService for Floor {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        VerticalExtent::try_new(
            object.clone(),
            ElevationInterval::exact(0.0)?,
            ElevationInterval::exact(3.0)?,
            Evidence::exact(source(), format!("extent:{}", object.local_id)),
        )
    }
}

impl MetricRoutingService for Floor {
    fn route(
        &self,
        request: &MetricRouteRequest,
    ) -> Result<MetricRouteOutcome, MetricRoutingError> {
        assert_eq!(request.origin().subject().local_id, "room");
        let to = &request.destination().subject().local_id;
        match self
            .walks
            .get(to)
            .unwrap_or_else(|| panic!("unexpected walk to {to}"))
        {
            Walk::Reachable(lower, upper) => {
                Ok(MetricRouteOutcome::Reachable(MetricRouteEvidence::try_new(
                    LengthInterval::try_new(*lower, *upper)?,
                    vec![request.origin().clone(), request.destination().clone()],
                    vec![id("room")],
                    Evidence::exact(source(), format!("walk:{to}")),
                )?))
            }
            Walk::Blocked => Ok(MetricRouteOutcome::Blocked(
                BlockedMetricRouteEvidence::new(
                    request.clone(),
                    CompleteMetricEvidence::try_new(Evidence::exact(
                        source(),
                        format!("blocked:{to}"),
                    ))?,
                ),
            )),
            Walk::Refused => Err(MetricRoutingError::Unavailable(
                "a gap is too narrow".into(),
            )),
        }
    }
}

/// A room with extinguishers stating `Units`, each `(local, units, plan
/// distance, walk)`.
fn extinguishers(around: &[(&str, i64, (f64, f64), Walk)]) -> (Model, Floor) {
    let mut model = Model::default().object("room", "space");
    let mut floor = Floor::default();
    for (local, units, plan, walk) in around {
        model = model.object(local, "extinguisher").value(
            local,
            "Pset_Extinguisher",
            "Units",
            PropertyValue::Integer(*units),
        );
        floor = floor.counterpart(local, *plan, *walk);
    }
    (model, floor)
}

/// 2 units walked 5 m to, 6 walked `ten`, 2 within 6 m in plan but walked
/// 20 m around a wall, 6 behind a locked wall 2 m away in plan, and 6 at
/// 30 m in plan, which no walk within 15 m reaches and is never walked to.
fn floor(ten: Walk) -> (Model, Floor) {
    extinguishers(&[
        ("near", 2, (3.0, 3.0), Walk::Reachable(5.0, 5.0)),
        ("ten", 6, (4.0, 4.0), ten),
        ("around", 2, (6.0, 6.0), Walk::Reachable(20.0, 20.0)),
        ("locked", 6, (2.0, 2.0), Walk::Blocked),
        ("farthest", 6, (30.0, 30.0), Walk::Reachable(31.0, 31.0)),
    ])
}

fn walking(aggregate: &str, radius: f64) -> String {
    format!(
        "{aggregate}_within;to=extinguisher;value=Pset_Extinguisher/Units;radius={radius};\
         measure=walking;walking_radius=0.3;walking_height=2;walking_step=0.02"
    )
}

fn measured(ten: Walk, name: &str) -> Result<Option<common::Cited>, String> {
    let (model, floor) = floor(ten);
    let (project, mut services) = model.services();
    floor.register(&mut services);
    common::measured_cited(&services, &project, &id("room"), name)
}

/// The units within a 15 m walk are those walked to within it: the
/// extinguisher 6 m away in plan but 20 m around a wall and the one behind
/// a wall no walk passes are not, though a radius would count both.
#[test]
fn values_within_a_walk_sum_the_counterparts_it_reaches() {
    let ten = Walk::Reachable(10.0, 10.0);
    assert_eq!(
        measured(ten, &walking("sum", 15.0)),
        Ok(Some(((8.0, 8.0), true)))
    );
    assert_eq!(
        measured(ten, &walking("min", 15.0)),
        Ok(Some(((2.0, 2.0), true)))
    );
    assert_eq!(
        measured(ten, &walking("max", 15.0)),
        Ok(Some(((6.0, 6.0), true)))
    );
    assert_eq!(
        measured(ten, &walking("mean", 15.0)),
        Ok(Some(((4.0, 4.0), true)))
    );
    // A walk of 20 m reaches the one around the wall; `from` leaves the
    // nearer ones out.
    assert_eq!(
        measured(ten, &walking("sum", 25.0)),
        Ok(Some(((10.0, 10.0), true)))
    );
    assert_eq!(
        measured(ten, &format!("{};from=7", walking("sum", 15.0))),
        Ok(Some(((6.0, 6.0), true)))
    );
}

/// A walk straddling the radius, or one the route service cannot bound,
/// adds its value or nothing, as a straddling distance does.
#[test]
fn a_walk_straddling_the_radius_widens_the_value() {
    for ten in [Walk::Reachable(14.0, 16.0), Walk::Refused] {
        assert_eq!(
            measured(ten, &walking("sum", 15.0)),
            Ok(Some(((2.0, 8.0), true))),
            "{ten:?}"
        );
        assert_eq!(
            measured(ten, &walking("max", 15.0)),
            Ok(Some(((2.0, 6.0), true))),
            "{ten:?}"
        );
        let mean = measured(ten, &walking("mean", 15.0)).unwrap_err();
        assert!(mean.contains("may or may not be within"), "{mean}");
    }
    // Wholly within or beyond, it decides again.
    assert_eq!(
        measured(Walk::Reachable(14.0, 16.0), &walking("sum", 16.0)),
        Ok(Some(((8.0, 8.0), true)))
    );
    assert_eq!(
        measured(Walk::Reachable(14.0, 16.0), &walking("sum", 13.0)),
        Ok(Some(((2.0, 2.0), true)))
    );
}

/// Walking needs its profile, and is never projected; no other measure
/// takes the profile.
#[test]
fn a_walk_declares_its_profile_and_nothing_projected() {
    use axioval_ir::measured::parse;
    let base = "sum_within;to=extinguisher;value=Pset_Extinguisher/Units;radius=15";
    for (written, expected) in [
        (
            format!("{base};measure=walking;walking_radius=0.3;walking_height=2"),
            "`sum_within` needs `walking_step`",
        ),
        (
            format!("{base};walking_radius=0.3"),
            "`sum_within` parameter `walking_radius`: only `measure=walking` takes it",
        ),
        (
            format!("{};projection=horizontal", walking("sum", 15.0)),
            "`sum_within` parameter `projection`: a walk is measured in plan, never projected",
        ),
    ] {
        assert_eq!(parse(&written).unwrap_err().to_string(), expected);
    }
    assert!(parse(&walking("mean", 15.0)).is_ok());
}

/// Rewritten as an expression rule, the units within a 15 m walk hold
/// against 8 and fail against 9; a walk straddling the radius decides only
/// the bounds it cannot change.
#[test]
fn an_expression_rule_judges_the_sum_within_a_walk() {
    let judge = |ten: Walk, required: f64| {
        let (model, floor) = floor(ten);
        let floor = Arc::new(floor);
        let requirement = serde_json::json!({"kind": "compare", "operator": "greaterThanOrEquals",
            "left": {"kind": "property", "propertySet": "axioval:measured",
                "property": walking("sum", 15.0)},
            "right": {"kind": "literal", "value": {"type": "number", "value": required}}});
        let rule = rule(
            "axioval:capability.expression",
            kind("space"),
            vec![("requirement", common::expression(requirement))],
        );
        let outcome =
            model.evaluate_measured(&axioval_rules::ExpressionRequirement, &rule, |services| {
                services
                    .register(ProximityServiceHandle::new(floor.clone()))
                    .unwrap();
                services
                    .register(PlanSpanServiceHandle::new(floor.clone()))
                    .unwrap();
                services
                    .register(VerticalExtentServiceHandle::new(floor.clone()))
                    .unwrap();
                services
                    .register(MetricRoutingServiceHandle::new(floor.clone()))
                    .unwrap();
            });
        if !outcome.findings().is_empty() {
            "fails"
        } else if outcome.not_evaluated_outcomes().is_empty() {
            "holds"
        } else {
            "open"
        }
    };
    let ten = Walk::Reachable(10.0, 10.0);
    let straddling = Walk::Reachable(14.0, 16.0);
    assert_eq!(judge(ten, 8.0), "holds");
    assert_eq!(judge(ten, 9.0), "fails");
    assert_eq!(judge(straddling, 8.0), "open");
    assert_eq!(judge(straddling, 2.0), "holds");
    assert_eq!(judge(straddling, 9.0), "fails");
}
