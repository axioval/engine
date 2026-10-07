//! `space-distance`: the nearest destination space in a straight line or
//! walking.
//!
//! The stubs answer only what a test declares and panic on any other span
//! or route, which also proves each measure reaches the service it names.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    Bounds3, CapabilityEvaluation, CentrePlacement, CompleteMetricEvidence, ElevationInterval,
    GeometryFidelity, LengthInterval, MetricRouteOutcome, MetricRouteRequest, MetricRoutingError,
    MetricRoutingService, MetricRoutingServiceHandle, NearestTargetEvidence, NearestTargetOutcome,
    NearestTargetRequest, ObjectBounds, PlanCentre, PlanLength, PlanSpan, PlanSpanError,
    PlanSpanService, PlanSpanServiceHandle, ProjectedDistanceEvidence, ProximityError,
    ProximityEvidence, ProximityProjection, ProximityRequest, ProximityService,
    ProximityServiceHandle, ServiceRegistry, UnreachableTargetsEvidence, VerticalExtent,
    VerticalExtentError, VerticalExtentService, VerticalExtentServiceHandle,
};
use axioval_ir::contract::{ParameterValue, Selector, TableRow};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId};
use axioval_rules::SpaceDistance;

/// `space-distance` as it runs, held to the implementation it replaced on
/// every evaluation.
static HELD: common::Held = common::Held(&SpaceDistance, &axioval_rules::reference::SpaceDistance);
use common::{
    Model, assert_deviation, boolean, deviation_of, findings, id, kind, number, rule, selector,
    source, string, strings, unevaluated,
};

const ID: &str = "axioval:capability.space-distance";
const ADJACENT: &str = "axioval:derived.adjacent-space";

fn evidence(lower: f64, upper: f64, locator: String) -> Evidence {
    #[allow(clippy::float_cmp)]
    let exact = lower == upper;
    Evidence {
        source: source(),
        locator,
        exact,
    }
}

/// How a stubbed route between two spaces ends.
#[derive(Clone, Copy, Debug)]
enum Route {
    Reachable(f64, f64),
    Blocked,
    Refused,
}

/// Plan spans, centres, floors and routes a test declares.
#[derive(Default)]
struct Geometry {
    /// Unordered pair -> centre-to-centre interval.
    spans: BTreeMap<(String, String), (f64, f64)>,
    /// Centres placed outside their footprint.
    outside: Vec<String>,
    /// `(from, to)` -> how the route ends.
    routes: BTreeMap<(String, String), Route>,
    /// Unordered pair -> closest distance between the bodies.
    gaps: BTreeMap<(String, String), (f64, f64)>,
}

impl Geometry {
    fn span(mut self, a: &str, b: &str, lower: f64, upper: f64) -> Self {
        let key = if a < b { (a, b) } else { (b, a) };
        self.spans
            .insert((key.0.into(), key.1.into()), (lower, upper));
        self
    }

    fn gap(mut self, a: &str, b: &str, lower: f64, upper: f64) -> Self {
        let key = if a < b { (a, b) } else { (b, a) };
        self.gaps
            .insert((key.0.into(), key.1.into()), (lower, upper));
        self
    }

    fn route(mut self, from: &str, to: &str, route: Route) -> Self {
        self.routes.insert((from.into(), to.into()), route);
        self
    }

    fn outside(mut self, space: &str) -> Self {
        self.outside.push(space.into());
        self
    }

    fn register(self, services: &mut ServiceRegistry) {
        let shared = Arc::new(self);
        services
            .register(PlanSpanServiceHandle::new(shared.clone()))
            .unwrap();
        services
            .register(VerticalExtentServiceHandle::new(shared.clone()))
            .unwrap();
        services
            .register(MetricRoutingServiceHandle::new(shared.clone()))
            .unwrap();
        services
            .register(ProximityServiceHandle::new(shared))
            .unwrap();
    }
}

impl PlanSpanService for Geometry {
    fn measure_diameter(&self, object: &ObjectId) -> Result<PlanLength, PlanSpanError> {
        panic!("unexpected diameter of {object}")
    }

    fn measure_span(
        &self,
        first: &ObjectId,
        second: &ObjectId,
        between: PlanSpan,
    ) -> Result<PlanLength, PlanSpanError> {
        assert_eq!(between, PlanSpan::Centres);
        let (a, b) = (first.local_id.clone(), second.local_id.clone());
        let key = if a < b { (a, b) } else { (b, a) };
        let (lower, upper) = *self
            .spans
            .get(&key)
            .unwrap_or_else(|| panic!("unexpected span {key:?}"));
        // A pair declared not a number cannot be measured.
        if lower.is_nan() {
            return Err(PlanSpanError::Unavailable("no footprint".into()));
        }
        PlanLength::try_new(
            lower,
            upper,
            evidence(
                lower,
                upper,
                format!("plan-span:centres:{}:{}", key.0, key.1),
            ),
        )
    }

    fn measure_centre(&self, object: &ObjectId) -> Result<PlanCentre, PlanSpanError> {
        let placement = if self.outside.contains(&object.local_id) {
            CentrePlacement::Outside
        } else {
            CentrePlacement::Inside
        };
        PlanCentre::try_new(
            object.clone(),
            [1.0, 2.0],
            0.0,
            placement,
            evidence(0.0, 0.0, format!("plan-centre:{}", object.local_id)),
        )
    }
}

impl ProximityService for Geometry {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        ObjectBounds::try_new(
            object.clone(),
            Bounds3::try_new([0.0; 3], [1.0; 3])?,
            GeometryFidelity::Exact,
        )
    }

    fn measure_proximity(&self, _: &ProximityRequest) -> Result<ProximityEvidence, ProximityError> {
        panic!("space distance measures through measure_distance")
    }

    fn measure_distance(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProjectedDistanceEvidence, ProximityError> {
        assert!(matches!(
            request.projection(),
            ProximityProjection::Minimum3d
        ));
        let (a, b) = (
            request.subject().local_id.clone(),
            request.counterpart().local_id.clone(),
        );
        let key = if a < b { (a, b) } else { (b, a) };
        let (lower, upper) = *self
            .gaps
            .get(&key)
            .unwrap_or_else(|| panic!("unexpected closest distance {key:?}"));
        if lower.is_nan() {
            return Err(ProximityError::Unavailable);
        }
        #[allow(clippy::float_cmp)]
        let fidelity = if lower == upper {
            GeometryFidelity::Exact
        } else {
            GeometryFidelity::tessellated((upper - lower) / 2.0)?
        };
        ProjectedDistanceEvidence::try_new(
            request.clone(),
            lower,
            upper,
            fidelity,
            evidence(lower, upper, format!("closest:{}:{}", key.0, key.1)),
        )
    }
}

impl VerticalExtentService for Geometry {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        VerticalExtent::try_new(
            object.clone(),
            ElevationInterval::exact(0.0)?,
            ElevationInterval::exact(3.0)?,
            evidence(0.0, 0.0, format!("extent:{}", object.local_id)),
        )
    }
}

impl MetricRoutingService for Geometry {
    fn climbs_connectors(&self) -> bool {
        true
    }

    fn route(
        &self,
        request: &MetricRouteRequest,
    ) -> Result<MetricRouteOutcome, MetricRoutingError> {
        panic!("walking asks for nearest targets, not the route {request:?}")
    }

    /// Answers from the declared pair routes as the Axiolid backend would:
    /// the nearest reachable target bounds from above; a refused one is
    /// unmeasured and drops the lower bound to zero; only blocked ones make
    /// the targets unreachable.
    fn nearest_target(
        &self,
        request: &NearestTargetRequest,
    ) -> Result<NearestTargetOutcome, MetricRoutingError> {
        // Every walk starts at the stubbed centre, on the floor.
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(request.origin().coordinates_metres(), [1.0, 2.0, 0.0]);
        }
        let from = request.origin().subject().local_id.clone();
        let mut best: Option<(usize, f64, f64)> = None;
        let mut refused = false;
        // A walk climbing connectors is declared as `{target}^{connectors}`.
        let climbed = request.connectors().map_or_else(String::new, |routing| {
            let names: Vec<&str> = routing
                .connectors()
                .iter()
                .map(|connector| connector.object().local_id.as_str())
                .collect();
            format!("^{}", names.join(","))
        });
        for (index, target) in request.targets().iter().enumerate() {
            let key = (
                from.clone(),
                format!("{}{climbed}", target.subject().local_id),
            );
            match self
                .routes
                .get(&key)
                .unwrap_or_else(|| panic!("unexpected route {key:?}"))
            {
                Route::Reachable(lower, upper) => {
                    if best.is_none_or(|(_, _, least)| *upper < least) {
                        best = Some((index, best.map_or(*lower, |b| b.1.min(*lower)), *upper));
                    } else if let Some(found) = &mut best {
                        found.1 = found.1.min(*lower);
                    }
                }
                Route::Blocked => {}
                Route::Refused => refused = true,
            }
        }
        match best {
            Some((index, lower, upper)) => {
                let lower = if refused { 0.0 } else { lower };
                let to = &request.targets()[index];
                Ok(NearestTargetOutcome::Reached(
                    NearestTargetEvidence::try_new(
                        index,
                        LengthInterval::try_new(lower, upper)?,
                        vec![request.origin().clone(), to.clone()],
                        Evidence::exact(
                            source(),
                            format!("route:{from}:{}", to.subject().local_id),
                        ),
                    )?,
                ))
            }
            None if refused => Err(MetricRoutingError::Unavailable(
                "a gap is too narrow".into(),
            )),
            None => Ok(NearestTargetOutcome::Unreachable(
                UnreachableTargetsEvidence::new(
                    request.clone(),
                    CompleteMetricEvidence::try_new(Evidence::exact(
                        source(),
                        format!("blocked:{from}"),
                    ))?,
                ),
            )),
        }
    }
}

fn row(cells: &[(&str, ParameterValue)]) -> TableRow {
    cells
        .iter()
        .map(|(column, value)| ((*column).to_owned(), value.clone()))
        .collect()
}

fn toilets(measure: &str, bounds: &[(&str, f64)]) -> TableRow {
    let mut cells = vec![
        ("from", selector(kind("office"))),
        ("to", selector(kind("toilet"))),
        ("measure", string(measure)),
    ];
    cells.extend(
        bounds
            .iter()
            .map(|(column, value)| (*column, number(*value))),
    );
    row(&cells)
}

fn walking() -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("walking_radius", number(0.3)),
        ("walking_height", number(2.0)),
        ("walking_step", number(0.02)),
    ]
}

fn spaces() -> Selector {
    Selector::AnyOf {
        operands: vec![kind("office"), kind("toilet")],
    }
}

fn run(
    model: Model,
    geometry: Geometry,
    rows: Vec<TableRow>,
    extra: Vec<(&'static str, ParameterValue)>,
) -> CapabilityEvaluation {
    let mut parameters = vec![("distances", ParameterValue::Table { value: rows })];
    parameters.extend(extra);
    model.evaluate_with(&HELD, &rule(ID, spaces(), parameters), |services| {
        geometry.register(services);
    })
}

/// Offices `o1` to `o3` and toilets `t1`, `t2`.
fn offices() -> Model {
    Model::default()
        .object("o1", "office")
        .object("o2", "office")
        .object("o3", "office")
        .object("t1", "toilet")
        .object("t2", "toilet")
}

#[test]
fn the_nearest_destination_in_a_straight_line_is_judged_as_an_interval() {
    let geometry = Geometry::default()
        // o1 has a toilet within 20 m.
        .span("o1", "t1", 15.0, 15.0)
        .span("o1", "t2", 30.0, 30.0)
        // o2's nearest may be 18 or 22 m away.
        .span("o2", "t1", 25.0, 25.0)
        .span("o2", "t2", 18.0, 22.0)
        // o3's are both too far.
        .span("o3", "t1", 25.0, 25.0)
        .span("o3", "t2", 30.0, 30.0);
    let evaluation = run(
        offices(),
        geometry,
        vec![toilets("straight", &[("maximum", 20.0)])],
        Vec::new(),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "o3".into(),
            format!(
                "the nearest destination, {}, is 25 m away in a straight line between centres; \
                 row 0 allows at most 20 m",
                id("t1")
            )
        )]
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("o2".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert_eq!(
        evaluation.not_evaluated_outcomes()[0].message(),
        "space-distance row 0: the nearest destination is at most 22 m in a straight line \
         between centres, at most 20 m required, and it is known only to lie between 18 and \
         22 m away"
    );
    assert!(
        evaluation.findings()[0]
            .evidence
            .iter()
            .any(|item| item.locator == "plan-span:centres:o3:t1")
    );
}

#[test]
fn a_walking_distance_too_long_is_found() {
    let only_o1 = || {
        Model::default()
            .object("o1", "office")
            .object("t1", "toilet")
            .object("t2", "toilet")
    };
    // In a straight line t1 is close; walking round the core it is 21 m or
    // more, and t2 cannot be reached at all.
    let geometry = || {
        Geometry::default()
            .route("o1", "t1", Route::Reachable(21.0, 22.5))
            .route("o1", "t2", Route::Blocked)
    };
    let evaluation = run(
        only_o1(),
        geometry(),
        vec![toilets("walking", &[("maximum", 20.0)])],
        walking(),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "o1".into(),
            format!(
                "the nearest destination, {}, is between 21 and 22.5 m away walking; row 0 \
                 allows at most 20 m",
                id("t1")
            )
        )]
    );
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");
    assert!(
        evaluation.findings()[0]
            .evidence
            .iter()
            .any(|item| item.locator == "route:o1:t1")
    );
    // 21 to 22.5 m against 20 m: 5 % to 12.5 % too far.
    assert_deviation(deviation_of(&evaluation, "the nearest"), (0.05, 0.125));

    // Within 25 m it passes: the upper bound meets the maximum.
    let evaluation = run(
        only_o1(),
        geometry(),
        vec![toilets("walking", &[("maximum", 25.0)])],
        walking(),
    );
    assert!(evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty());

    // A blocked route is no destination at all.
    let evaluation = run(
        Model::default()
            .object("o1", "office")
            .object("t2", "toilet"),
        geometry(),
        vec![toilets("walking", &[("maximum", 25.0)])],
        walking(),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "o1".into(),
            "reaches none of its 1 destination(s) walking; row 0 requires one within 25 m".into()
        )]
    );
}

#[test]
fn a_walk_climbs_the_selected_stair_to_another_storey() {
    // t1 is upstairs: only a walk up the stair reaches it.
    let model = || {
        Model::default()
            .object("o1", "office")
            .object("t1", "toilet")
            .object("stair", "stair")
    };
    let geometry = || Geometry::default().route("o1", "t1^stair", Route::Reachable(21.0, 22.5));
    let mut climbing = walking();
    climbing.push(("stair_selector", selector(kind("stair"))));
    climbing.push(("stair_length", string("horizontal-plus-vertical")));
    climbing.push(("vertical_factor", number(2.0)));
    let evaluation = run(
        model(),
        geometry(),
        vec![toilets("walking", &[("maximum", 20.0)])],
        climbing.clone(),
    );
    assert_eq!(findings(&evaluation).len(), 1, "{evaluation:?}");
    let evaluation = run(
        model(),
        geometry(),
        vec![toilets("walking", &[("maximum", 25.0)])],
        climbing,
    );
    assert!(evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty());
}

#[test]
fn a_refused_route_decides_only_what_it_cannot_change() {
    let model = Model::default()
        .object("o1", "office")
        .object("t1", "toilet")
        .object("t2", "toilet");
    let geometry = || {
        Geometry::default()
            .route("o1", "t1", Route::Reachable(12.0, 12.5))
            .route("o1", "t2", Route::Refused)
    };
    // A route within 15 m stands whatever the refused one would be.
    let passed = run(
        Model::default()
            .object("o1", "office")
            .object("t1", "toilet")
            .object("t2", "toilet"),
        geometry(),
        vec![toilets("walking", &[("maximum", 15.0)])],
        walking(),
    );
    assert!(passed.findings().is_empty() && unevaluated(&passed).is_empty());
    // Beyond 10 m the refused route might still be nearer.
    let evaluation = run(
        model,
        geometry(),
        vec![toilets("walking", &[("maximum", 10.0)])],
        walking(),
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("o1".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(
        message.contains("the nearest destination is at most 12.5 m walking")
            && message.contains("between 0 and 12.5 m away"),
        "{message}"
    );
}

#[test]
fn a_space_whose_centre_lies_outside_it_is_not_walked_from() {
    let evaluation = run(
        Model::default()
            .object("o1", "office")
            .object("t1", "toilet"),
        Geometry::default().outside("o1"),
        vec![toilets("walking", &[("maximum", 20.0)])],
        walking(),
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("o1".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert!(
        evaluation.not_evaluated_outcomes()[0]
            .message()
            .contains("lies outside its footprint"),
        "{evaluation:?}"
    );
}

#[test]
fn a_destination_too_close_is_found() {
    let evaluation = run(
        Model::default()
            .object("o1", "office")
            .object("t1", "toilet")
            .object("t2", "toilet"),
        Geometry::default()
            .span("o1", "t1", 3.0, 3.0)
            .span("o1", "t2", 9.0, 9.0),
        vec![toilets("straight", &[("minimum", 5.0)])],
        Vec::new(),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "o1".into(),
            format!(
                "{} is 3 m away in a straight line between centres; row 0 requires at least 5 m",
                id("t1")
            )
        )]
    );
    assert_deviation(deviation_of(&evaluation, &id("t1").to_string()), (0.4, 0.4));
}

#[test]
fn destinations_are_filtered_by_storey_and_direct_access() {
    // o1 and t1 on storey s1, t2 on s2; door d joins o1 to t2 only.
    let model = Model::default()
        .object("s1", "storey")
        .object("s2", "storey")
        .object("o1", "office")
        .object("t1", "toilet")
        .object("t2", "toilet")
        .object("d", "door")
        .edge("contains", "s1", "o1")
        .edge("contains", "s1", "t1")
        .edge("contains", "s2", "t2")
        .edge(ADJACENT, "d", "o1")
        .cite(
            ADJACENT,
            "d",
            &format!(
                "{ADJACENT};reach=1:{}->{}:side=+(1,0):entered=0",
                id("d"),
                id("o1")
            ),
        )
        .edge(ADJACENT, "d", "t2")
        .cite(
            ADJACENT,
            "d",
            &format!(
                "{ADJACENT};reach=1:{}->{}:side=-(1,0):entered=0",
                id("d"),
                id("t2")
            ),
        );
    let geometry = || {
        Geometry::default()
            .span("o1", "t1", 30.0, 30.0)
            .span("o1", "t2", 5.0, 5.0)
    };
    let storeys = || {
        vec![
            ("storey_path", strings(&["contains:backward"])),
            ("storey_selector", selector(kind("storey"))),
        ]
    };
    let access = || {
        vec![
            ("access_path", strings(&[ADJACENT])),
            ("door_selector", selector(kind("door"))),
        ]
    };
    let mut same_storey = toilets("straight", &[("maximum", 20.0)]);
    same_storey.insert("same_storey".into(), boolean(true));
    let evaluation = run(
        Model::default()
            .object("s1", "storey")
            .object("s2", "storey")
            .object("o1", "office")
            .object("t1", "toilet")
            .object("t2", "toilet")
            .edge("contains", "s1", "o1")
            .edge("contains", "s1", "t1")
            .edge("contains", "s2", "t2"),
        geometry(),
        vec![same_storey],
        storeys(),
    );
    // Only t1 shares o1's storey, and it is 30 m away.
    assert_eq!(
        findings(&evaluation),
        [(
            "o1".into(),
            format!(
                "the nearest destination on its storey, {}, is 30 m away in a straight line \
                 between centres; row 0 allows at most 20 m",
                id("t1")
            )
        )]
    );

    let mut direct = toilets("straight", &[("maximum", 20.0)]);
    direct.insert("direct_access".into(), boolean(true));
    let evaluation = run(model, geometry(), vec![direct], access());
    // t2 is reached through d and lies 5 m away.
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");
}

#[test]
fn declarations_that_cannot_be_judged_are_refused() {
    for (rows, extra) in [
        (vec![toilets("walking", &[("maximum", 1.0)])], Vec::new()),
        (vec![toilets("flying", &[("maximum", 1.0)])], Vec::new()),
        (vec![toilets("straight", &[])], Vec::new()),
        (
            vec![toilets("straight", &[("minimum", 5.0), ("maximum", 1.0)])],
            Vec::new(),
        ),
        (
            {
                let mut row = toilets("straight", &[("maximum", 1.0)]);
                row.insert("same_storey".into(), boolean(true));
                vec![row]
            },
            Vec::new(),
        ),
        (
            {
                let mut row = toilets("straight", &[("maximum", 1.0)]);
                row.insert("direct_access".into(), boolean(true));
                vec![row]
            },
            Vec::new(),
        ),
    ] {
        let evaluation = run(offices(), Geometry::default(), rows, extra);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)],
        );
    }
}

#[test]
fn a_distance_between_tessellated_bodies_is_cited_inexactly() {
    // The measured list `distance_rows` states the nearest distance exact
    // only on exact evidence: a gap between tessellated bodies is an
    // interval whose finding is never exact, an exact one's always is.
    let evaluate = |gap: (f64, f64)| {
        run(
            Model::default()
                .object("o1", "office")
                .object("t1", "toilet"),
            Geometry::default().gap("o1", "t1", gap.0, gap.1),
            vec![toilets("closest", &[("minimum", 0.5)])],
            Vec::new(),
        )
    };
    let tessellated = evaluate((0.1, 0.2));
    assert_eq!(tessellated.findings().len(), 1);
    assert!(
        tessellated.findings()[0]
            .evidence
            .iter()
            .any(|item| !item.exact)
    );
    let exact = evaluate((0.2, 0.2));
    assert!(exact.findings()[0].evidence.iter().all(|item| item.exact));
}

#[test]
fn the_closest_distance_is_measured_between_the_bodies() {
    // Two long rooms side by side: their centres lie 8 m apart, their bodies
    // only the 0.2 m wall between them.
    let rooms = || {
        Model::default()
            .object("o1", "office")
            .object("t1", "toilet")
    };
    let geometry = || {
        Geometry::default()
            .span("o1", "t1", 8.0, 8.0)
            .gap("o1", "t1", 0.2, 0.2)
    };
    let closest = run(
        rooms(),
        geometry(),
        vec![toilets("closest", &[("maximum", 1.0)])],
        Vec::new(),
    );
    assert!(
        closest.findings().is_empty() && unevaluated(&closest).is_empty(),
        "{closest:?}"
    );
    let straight = run(
        rooms(),
        geometry(),
        vec![toilets("straight", &[("maximum", 1.0)])],
        Vec::new(),
    );
    assert_eq!(
        findings(&straight),
        [(
            "o1".into(),
            format!(
                "the nearest destination, {}, is 8 m away in a straight line between centres; \
                 row 0 allows at most 1 m",
                id("t1")
            )
        )]
    );

    // Too close for a minimum, citing the measurement.
    let evaluation = run(
        rooms(),
        geometry(),
        vec![toilets("closest", &[("minimum", 0.5)])],
        Vec::new(),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "o1".into(),
            format!(
                "{} is 0.2 m away between the closest points of the bodies; row 0 requires at \
                 least 0.5 m",
                id("t1")
            )
        )]
    );
    assert!(
        evaluation.findings()[0]
            .evidence
            .iter()
            .any(|item| item.locator == "closest:o1:t1")
    );

    // A tessellated gap straddling the maximum decides nothing.
    let evaluation = run(
        rooms(),
        Geometry::default().gap("o1", "t1", 0.9, 1.1),
        vec![toilets("closest", &[("maximum", 1.0)])],
        Vec::new(),
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("o1".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn without_the_services_it_is_not_evaluated() {
    let evaluation = Model::default()
        .object("o1", "office")
        .object("t1", "toilet")
        .evaluate(
            &HELD,
            &rule(
                ID,
                spaces(),
                vec![(
                    "distances",
                    ParameterValue::Table {
                        value: vec![toilets("straight", &[("maximum", 20.0)])],
                    },
                )],
            ),
        );
    assert_eq!(
        unevaluated(&evaluation),
        [("o1".into(), NotEvaluatedReason::MissingService)]
    );
}

/// Generated offices and toilets: every pair's span, gap and route exact,
/// an interval or refused, toilets whose kind cannot be read, rows by each
/// measure with random bounds, held to the reference.
mod generated {
    use super::*;
    use proptest::prelude::*;

    /// A pair's measurement: an interval, or not a number when refused.
    fn measured() -> impl Strategy<Value = (f64, f64)> {
        prop_oneof![
            (0.0..40.0f64).prop_map(|value| (value, value)),
            (0.0..40.0f64, 0.0..6.0f64).prop_map(|(lower, width)| (lower, lower + width)),
            Just((f64::NAN, f64::NAN)),
        ]
    }

    fn route() -> impl Strategy<Value = Route> {
        prop_oneof![
            (0.0..60.0f64, 0.0..4.0f64)
                .prop_map(|(lower, width)| Route::Reachable(lower, lower + width)),
            Just(Route::Blocked),
            Just(Route::Refused),
        ]
    }

    const OFFICES: [&str; 2] = ["o1", "o2"];
    const TOILETS: [&str; 3] = ["t1", "t2", "t3"];

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn generated_distances_hold_parity(
            spans in proptest::collection::vec(measured(), 6),
            gaps in proptest::collection::vec(measured(), 6),
            routes in proptest::collection::vec(route(), 6),
            unreadable in proptest::option::of(0..3usize),
            measure in 0..3usize,
            minimum in proptest::option::of(0.0..20.0f64),
            maximum in proptest::option::of(10.0..45.0f64),
            two_rows in any::<bool>(),
        ) {
            let mut model = Model::default()
                .object("o1", "office")
                .object("o2", "office")
                .object("t1", "toilet")
                .object("t2", "toilet")
                .object("t3", "toilet");
            if let Some(toilet) = unreadable {
                model = model.unreadable(TOILETS[toilet]);
            }
            let mut geometry = Geometry::default();
            for (index, (office, toilet)) in OFFICES
                .iter()
                .flat_map(|office| TOILETS.iter().map(move |toilet| (*office, *toilet)))
                .enumerate()
            {
                geometry = geometry
                    .span(office, toilet, spans[index].0, spans[index].1)
                    .gap(office, toilet, gaps[index].0, gaps[index].1)
                    .route(office, toilet, routes[index]);
            }
            let measure = ["straight", "closest", "walking"][measure];
            let mut bounds = Vec::new();
            if let Some(minimum) = minimum {
                bounds.push(("minimum", minimum));
            }
            match maximum {
                Some(maximum) if minimum.is_none_or(|minimum| minimum <= maximum) => {
                    bounds.push(("maximum", maximum));
                }
                _ if bounds.is_empty() => bounds.push(("maximum", 30.0)),
                _ => {}
            }
            let mut rows = vec![toilets(measure, &bounds)];
            if two_rows {
                rows.push(toilets("straight", &[("maximum", 25.0)]));
            }
            let extra = if measure == "walking" {
                walking()
            } else {
                Vec::new()
            };
            run(model, geometry, rows, extra);
        }
    }
}
