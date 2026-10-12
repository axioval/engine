//! `sum_within`, `min_within`, `max_within` and `mean_within` over the
//! counterparts within a walking distance, or sharing a derived group.
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

/// Fire compartments keyed by their least room: `r1` holds the room `r1`
/// and the extinguishers `e1` (2 units) and `e2` (6), `r2` the room `r2`
/// and `e3` (2), `r3` the room `r3` and `e5` (6); the room `r4` is in none.
/// With `merging`, `r1` and `r2` may be one compartment; with `undecided`,
/// whether `r5` is in one is.
fn compartments(merging: bool, undecided: bool) -> axioval_engine::DerivedGroups {
    use axioval_engine::{Grouping, Membership, group_id};
    let group = |key: &str| group_id(&source(), "compartments", key).unwrap();
    let grouped = |key: &str| Membership::Grouped(group(key), vec![]);
    let mut members = vec![
        (id("r1"), grouped("r1")),
        (id("e1"), grouped("r1")),
        (id("e2"), grouped("r1")),
        (id("r2"), grouped("r2")),
        (id("e3"), grouped("r2")),
        (id("r3"), grouped("r3")),
        (id("e5"), grouped("r3")),
        (id("r4"), Membership::Ungrouped(vec![])),
    ];
    if undecided {
        members.push((
            id("r5"),
            Membership::Undecided(
                axioval_ir::NotEvaluatedReason::IncompleteEvidence,
                "its extent cannot be read".into(),
            ),
        ));
    }
    let keys = ["r1", "r2", "r3"]
        .into_iter()
        .map(|key| (group(key), key.to_owned()))
        .collect();
    let mut grouping = Grouping::of("compartments", members, &keys);
    if merging {
        let why = "a wall's fire rating is unreadable";
        grouping.undecide(&group("r1"), why);
        grouping.undecide(&group("r2"), why);
    }
    axioval_engine::DerivedGroups::default().with_grouping("compartments", grouping)
}

fn compartment_model() -> Model {
    let mut model = Model::default();
    for room in ["r1", "r2", "r3", "r4", "r5"] {
        model = model.object(room, "space");
    }
    for (local, units) in [("e1", 2), ("e2", 6), ("e3", 2), ("e5", 6)] {
        model = model.object(local, "extinguisher").value(
            local,
            "Pset_Extinguisher",
            "Units",
            PropertyValue::Integer(units),
        );
    }
    model
}

fn grouped(aggregate: &str) -> String {
    format!("{aggregate}_within;to=extinguisher;value=Pset_Extinguisher/Units;group=compartments")
}

fn in_compartment(
    (merging, undecided): (bool, bool),
    object: &ObjectId,
    name: &str,
) -> Result<Option<common::Cited>, String> {
    let (project, mut services) = compartment_model().services();
    services
        .register(Arc::new(compartments(merging, undecided)))
        .unwrap();
    common::measured_cited(&services, &project, object, name)
}

/// The extinguisher units per fire compartment: those sharing a room's
/// compartment, or the members of the compartment itself.
#[test]
fn values_within_a_group_sum_the_counterparts_sharing_it() {
    let decided = (false, false);
    assert_eq!(
        in_compartment(decided, &id("r1"), &grouped("sum")),
        Ok(Some(((8.0, 8.0), true)))
    );
    assert_eq!(
        in_compartment(decided, &id("r1"), &grouped("min")),
        Ok(Some(((2.0, 2.0), true)))
    );
    assert_eq!(
        in_compartment(decided, &id("r1"), &grouped("mean")),
        Ok(Some(((4.0, 4.0), true)))
    );
    assert_eq!(
        in_compartment(decided, &id("r2"), &grouped("max")),
        Ok(Some(((2.0, 2.0), true)))
    );
    // The compartment itself sums its members.
    let first = axioval_engine::group_id(&source(), "compartments", "r1").unwrap();
    assert_eq!(
        in_compartment(decided, &first, &grouped("sum")),
        Ok(Some(((8.0, 8.0), true)))
    );
    // A room in no compartment shares none: a sum of none is zero.
    assert_eq!(
        in_compartment(decided, &id("r4"), &grouped("sum")),
        Ok(Some(((0.0, 0.0), true)))
    );
    assert_eq!(
        in_compartment(decided, &id("r4"), &grouped("max")),
        Ok(None)
    );
}

/// Where two compartments may be one, an extinguisher of the other may or
/// may not share the room's: it adds its value or nothing. One of a
/// compartment surely apart never counts.
#[test]
fn a_counterpart_whose_membership_is_undecided_widens_the_value() {
    let merging = (true, false);
    assert_eq!(
        in_compartment(merging, &id("r1"), &grouped("sum")),
        Ok(Some(((8.0, 10.0), true)))
    );
    assert_eq!(
        in_compartment(merging, &id("r2"), &grouped("sum")),
        Ok(Some(((2.0, 10.0), true)))
    );
    assert_eq!(
        in_compartment(merging, &id("r2"), &grouped("max")),
        Ok(Some(((2.0, 6.0), true)))
    );
    let mean = in_compartment(merging, &id("r1"), &grouped("mean")).unwrap_err();
    assert!(mean.contains("may or may not be within"), "{mean}");
    // The third stays apart.
    assert_eq!(
        in_compartment(merging, &id("r3"), &grouped("sum")),
        Ok(Some(((6.0, 6.0), true)))
    );
    // A room whose own compartment is undecided is not evaluated.
    let open = in_compartment((false, true), &id("r5"), &grouped("sum")).unwrap_err();
    assert!(open.contains("is undecided"), "{open}");
}

/// A group reaches the counterparts at no distance: either `radius` or
/// `group`, and nothing of a distance beside `group`.
#[test]
fn a_group_reach_states_no_distance() {
    use axioval_ir::measured::parse;
    let base = "sum_within;to=extinguisher;value=Pset_Extinguisher/Units";
    for (written, expected) in [
        (base.to_owned(), "`sum_within` needs `radius`"),
        (
            format!("{base};radius=15;group=compartments"),
            "`sum_within` parameter `group`: one of `radius`, `group`, `zone` reaches the \
             counterparts",
        ),
        (
            format!("{base};group=compartments;from=2"),
            "`sum_within` parameter `from`: a shared `group` reaches the counterparts at no \
             distance",
        ),
        (
            format!("{base};group=compartments;measure=walking"),
            "`sum_within` parameter `measure`: a shared `group` reaches the counterparts at \
             no distance",
        ),
    ] {
        assert_eq!(parse(&written).unwrap_err().to_string(), expected);
    }
    assert!(parse(&grouped("mean")).is_ok());
}

/// Rewritten as an expression rule, a room's compartment holds at least 8
/// units only where it decidedly does: `r1`'s holds, `r2`'s fails, and
/// with the two possibly one compartment, `r2`'s is open.
#[test]
fn an_expression_rule_judges_the_sum_within_a_group() {
    let judge = |merging: bool, room: &str| {
        let groups = Arc::new(compartments(merging, false));
        let requirement = serde_json::json!({"kind": "compare", "operator": "greaterThanOrEquals",
            "left": {"kind": "property", "propertySet": "axioval:measured",
                "property": grouped("sum")},
            "right": {"kind": "literal", "value": {"type": "number", "value": 8}}});
        let rule = rule(
            "axioval:capability.expression",
            axioval_ir::contract::Selector::Objects {
                objects: std::collections::BTreeSet::from([id(room)]),
            },
            vec![("requirement", common::expression(requirement))],
        );
        let outcome = compartment_model().evaluate_measured(
            &axioval_rules::ExpressionRequirement,
            &rule,
            |services| services.register(groups.clone()).unwrap(),
        );
        if !outcome.findings().is_empty() {
            "fails"
        } else if outcome.not_evaluated_outcomes().is_empty() {
            "holds"
        } else {
            "open"
        }
    };
    assert_eq!(judge(false, "r1"), "holds");
    assert_eq!(judge(false, "r2"), "fails");
    assert_eq!(judge(true, "r2"), "open");
    assert_eq!(judge(true, "r1"), "holds");
}
