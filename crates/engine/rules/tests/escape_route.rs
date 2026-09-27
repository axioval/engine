//! `escape-route`: travel distance, exits and exit widths per space use.
//!
//! The stubs answer only what a test declares and panic on anything else,
//! which also proves each check reaches the service it names.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, CentrePlacement, CompleteMetricEvidence, ElevationInterval,
    FarthestPointEvidence, FarthestPointOutcome, FarthestPointRequest, LengthInterval, MetricPoint,
    MetricRouteOutcome, MetricRouteRequest, MetricRoutingError, MetricRoutingService,
    MetricRoutingServiceHandle, NearestTargetEvidence, NearestTargetOutcome, NearestTargetRequest,
    NotEvaluatedReason, PlanArea, PlanAreaError, PlanAreaService, PlanAreaServiceHandle,
    PlanCentre, PlanLength, PlanSpan, PlanSpanError, PlanSpanService, PlanSpanServiceHandle,
    ServiceRegistry, UnreachableRegionEvidence, UnreachableTargetsEvidence, VerticalExtent,
    VerticalExtentError, VerticalExtentService, VerticalExtentServiceHandle,
};
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector, TableRow};
use axioval_ir::{Evidence, ObjectId, PropertyValue, QuantityDimension};
use axioval_rules::EscapeRoute;
use common::{
    Model, findings, id, integer, kind, number, property, rule, selector, source, string, strings,
    unevaluated,
};

const CAPABILITY: &str = "axioval:capability.escape-route";

/// How a stubbed walk ends.
#[derive(Clone, Copy)]
enum Walk {
    Between(f64, f64),
    Unreachable,
    Refused,
}

/// Areas, diagonals and walks a test declares. Walks are keyed by their
/// start (`region` or door) and the sorted names of their targets.
#[derive(Default)]
struct Geometry {
    areas: BTreeMap<String, (f64, f64)>,
    diameters: BTreeMap<String, f64>,
    walks: BTreeMap<(String, String), Walk>,
}

impl Geometry {
    fn area(mut self, local: &str, lower: f64, upper: f64) -> Self {
        self.areas.insert(local.into(), (lower, upper));
        self
    }

    fn diameter(mut self, local: &str, metres: f64) -> Self {
        self.diameters.insert(local.into(), metres);
        self
    }

    fn walk(mut self, from: &str, to: &str, walk: Walk) -> Self {
        self.walks.insert((from.into(), to.into()), walk);
        self
    }

    fn register(self, services: &mut ServiceRegistry) {
        let shared = Arc::new(self);
        services
            .register(PlanAreaServiceHandle::new(shared.clone()))
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

    fn lookup(&self, from: &ObjectId, targets: &[MetricPoint]) -> Walk {
        let mut names: Vec<&str> = targets
            .iter()
            .map(|target| target.subject().local_id.as_str())
            .collect();
        names.sort_unstable();
        let key = (from.local_id.clone(), names.join(","));
        *self
            .walks
            .get(&key)
            .unwrap_or_else(|| panic!("unexpected walk {key:?}"))
    }
}

fn exact(locator: String) -> Evidence {
    Evidence::exact(source(), locator)
}

impl PlanAreaService for Geometry {
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        let (lower, upper) = *self
            .areas
            .get(&object.local_id)
            .unwrap_or_else(|| panic!("unexpected area of {object}"));
        #[allow(clippy::float_cmp)]
        let exact = lower == upper;
        PlanArea::try_new(
            lower,
            upper,
            Evidence {
                source: source(),
                locator: format!("area:{}", object.local_id),
                exact,
            },
        )
    }

    fn measure_plan_overlap(&self, _: &ObjectId, _: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        panic!("escape routes measure no overlap")
    }
}

impl PlanSpanService for Geometry {
    fn measure_diameter(&self, object: &ObjectId) -> Result<PlanLength, PlanSpanError> {
        let metres = *self
            .diameters
            .get(&object.local_id)
            .unwrap_or_else(|| panic!("unexpected diameter of {object}"));
        PlanLength::try_new(
            metres,
            metres,
            exact(format!("diameter:{}", object.local_id)),
        )
    }

    fn measure_span(
        &self,
        _: &ObjectId,
        _: &ObjectId,
        _: PlanSpan,
    ) -> Result<PlanLength, PlanSpanError> {
        panic!("escape routes measure no span")
    }

    fn measure_centre(&self, object: &ObjectId) -> Result<PlanCentre, PlanSpanError> {
        PlanCentre::try_new(
            object.clone(),
            [1.0, 2.0],
            0.0,
            CentrePlacement::Inside,
            exact(format!("centre:{}", object.local_id)),
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
            ElevationInterval::exact(2.1)?,
            exact(format!("extent:{}", object.local_id)),
        )
    }
}

impl MetricRoutingService for Geometry {
    fn route(
        &self,
        request: &MetricRouteRequest,
    ) -> Result<MetricRouteOutcome, MetricRoutingError> {
        panic!("escape routes walk to the nearest exit, not the route {request:?}")
    }

    fn nearest_target(
        &self,
        request: &NearestTargetRequest,
    ) -> Result<NearestTargetOutcome, MetricRoutingError> {
        assert!(request.profile().radius_metres() == 0.0);
        let from = request.origin().subject();
        match self.lookup(from, request.targets()) {
            Walk::Between(lower, upper) => Ok(NearestTargetOutcome::Reached(
                NearestTargetEvidence::try_new(
                    0,
                    LengthInterval::try_new(lower, upper)?,
                    vec![request.origin().clone(), request.targets()[0].clone()],
                    exact(format!("nearest:{}", from.local_id)),
                )?,
            )),
            Walk::Unreachable => Ok(NearestTargetOutcome::Unreachable(
                UnreachableTargetsEvidence::new(
                    request.clone(),
                    CompleteMetricEvidence::try_new(exact(format!("cut-off:{}", from.local_id)))?,
                ),
            )),
            Walk::Refused => Err(MetricRoutingError::Unavailable(
                "a gap is too narrow".into(),
            )),
        }
    }

    fn farthest_point(
        &self,
        request: &FarthestPointRequest,
    ) -> Result<FarthestPointOutcome, MetricRoutingError> {
        assert!(request.profile().radius_metres() == 0.0);
        let region = request.region();
        let witness = MetricPoint::try_new(region.clone(), [19.0, 0.0, 0.0])?;
        match self.lookup(region, request.targets()) {
            Walk::Between(lower, upper) => Ok(FarthestPointOutcome::Bounded(
                FarthestPointEvidence::try_new(
                    LengthInterval::try_new(lower, upper)?,
                    witness,
                    upper - lower <= request.tolerance_metres(),
                    exact(format!("farthest:{}", region.local_id)),
                )?,
            )),
            Walk::Unreachable => Ok(FarthestPointOutcome::Unreachable(
                UnreachableRegionEvidence::new(
                    request.clone(),
                    witness,
                    CompleteMetricEvidence::try_new(exact(format!("cut-off:{}", region.local_id)))?,
                ),
            )),
            Walk::Refused => Err(MetricRoutingError::Unavailable(
                "a gap is too narrow".into(),
            )),
        }
    }
}

/// Hall `hall` with doors `d1` and `d2` on its boundary (`bounds` runs from
/// a door to its room).
fn model() -> Model {
    Model::default()
        .object("hall", "space")
        .object("d1", "door")
        .object("d2", "door")
        .edge("bounds", "d1", "hall")
        .edge("bounds", "d2", "hall")
}

fn use_row(cells: &[(&str, ParameterValue)]) -> TableRow {
    let mut row: TableRow = cells
        .iter()
        .map(|(column, value)| ((*column).to_owned(), value.clone()))
        .collect();
    row.insert("spaces".into(), selector(kind("space")));
    row
}

fn uses(cells: &[(&str, ParameterValue)]) -> (&'static str, ParameterValue) {
    (
        "uses",
        ParameterValue::Table {
            value: vec![use_row(cells)],
        },
    )
}

fn widths() -> (&'static str, ParameterValue) {
    let row = |occupants: i64, width: f64| -> TableRow {
        [
            ("occupants".to_owned(), integer(occupants)),
            ("width".to_owned(), number(width)),
        ]
        .into_iter()
        .collect()
    };
    (
        "widths",
        ParameterValue::Table {
            value: vec![row(20, 0.9), row(200, 1.2)],
        },
    )
}

fn exits(selector_value: Selector) -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("exit_path", strings(&["bounds:backward"])),
        ("exit_selector", selector(selector_value)),
        ("walking_height", number(2.0)),
        ("walking_step", number(0.02)),
    ]
}

fn evaluate(
    model: Model,
    geometry: Geometry,
    parameters: Vec<(&'static str, ParameterValue)>,
) -> CapabilityEvaluation {
    model.evaluate_with(
        &EscapeRoute,
        &rule(CAPABILITY, kind("space"), parameters),
        |services| {
            geometry.register(services);
        },
    )
}

fn with(
    mut parameters: Vec<(&'static str, ParameterValue)>,
    extra: Vec<(&'static str, ParameterValue)>,
) -> Vec<(&'static str, ParameterValue)> {
    parameters.extend(extra);
    parameters
}

#[test]
fn a_room_whose_farthest_point_exceeds_the_travel_distance_is_found() {
    let geometry = || Geometry::default().walk("hall", "d1,d2", Walk::Between(21.0, 21.01));
    let run = |maximum: f64| {
        evaluate(
            model(),
            geometry(),
            with(
                exits(kind("door")),
                vec![uses(&[("maximum_travel", number(maximum))])],
            ),
        )
    };
    let evaluation = run(20.0);
    assert_eq!(
        findings(&evaluation),
        [(
            "hall".into(),
            "its farthest point, around (19.00, 0.00), lies between 21 and 21.01 m from the \
             nearest exit walking; use 0 allows at most 20 m of travel"
                .into()
        )]
    );
    let finding = &evaluation.findings()[0];
    assert!(
        finding
            .evidence
            .iter()
            .any(|item| item.locator == "farthest:hall")
    );
    assert_eq!(finding.related, vec![id("d1"), id("d2")]);
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");

    // Within 25 m it passes; at 21.005 m the bracket straddles the limit.
    let evaluation = run(25.0);
    assert!(evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty());
    let evaluation = run(21.005);
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("hall".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn part_of_a_room_cut_off_from_every_exit_is_found() {
    let evaluation = evaluate(
        model(),
        Geometry::default().walk("hall", "d1,d2", Walk::Unreachable),
        with(
            exits(kind("door")),
            vec![uses(&[("maximum_travel", number(35.0))])],
        ),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "hall".into(),
            "part of it, around (19.00, 0.00), reaches no exit walking; use 0 allows at most \
             35 m of travel"
                .into()
        )]
    );
}

#[test]
fn a_room_with_too_few_exits_is_found() {
    let evaluation = evaluate(
        model(),
        Geometry::default(),
        with(exits(kind("door")), vec![uses(&[("exits", integer(3))])]),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "hall".into(),
            "has 2 exit(s) via bounds; use 0 requires at least 3".into()
        )]
    );
    let evaluation = evaluate(
        model(),
        Geometry::default(),
        with(exits(kind("door")), vec![uses(&[("exits", integer(2))])]),
    );
    assert!(evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty());
}

fn tagged_door() -> Selector {
    Selector::AllOf {
        operands: vec![
            kind("door"),
            Selector::Property {
                property_set: Some("Exit".into()),
                property: "IsExit".into(),
                operator: ComparisonOperator::Exists,
                value: None,
                case_sensitive: true,
                trim: false,
                quantifier: None,
                precision: None,
            },
        ],
    }
}

#[test]
fn an_undecided_exit_decides_only_what_it_cannot_change() {
    // d1 is a tagged exit; whether d2 is one cannot be read.
    let tagged = || {
        model()
            .value("d1", "Exit", "IsExit", PropertyValue::Boolean(true))
            .unreadable("d2")
    };
    // Two exits required: d2 might be the second.
    let evaluation = evaluate(
        tagged(),
        Geometry::default(),
        with(exits(tagged_door()), vec![uses(&[("exits", integer(2))])]),
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("hall".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    // Travel: through d1 alone at most 18 m, so it passes whatever d2 is;
    // with d2 at least 12 m, so 10 m fails whatever it is.
    let walks = || {
        Geometry::default()
            .walk("hall", "d1", Walk::Between(17.9, 18.0))
            .walk("hall", "d1,d2", Walk::Between(12.0, 12.1))
    };
    let travel = |maximum: f64| {
        evaluate(
            tagged(),
            walks(),
            with(
                exits(tagged_door()),
                vec![uses(&[("maximum_travel", number(maximum))])],
            ),
        )
    };
    let evaluation = travel(20.0);
    assert!(evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty());
    let evaluation = travel(10.0);
    assert_eq!(findings(&evaluation).len(), 1, "{evaluation:?}");
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");
    let evaluation = travel(15.0);
    assert!(evaluation.findings().is_empty());
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(
        message.contains("between 12 and 18 m walking") && message.contains("whether"),
        "{message}"
    );
}

#[test]
fn a_refused_walk_leaves_travel_undecided() {
    let evaluation = evaluate(
        model(),
        Geometry::default().walk("hall", "d1,d2", Walk::Refused),
        with(
            exits(kind("door")),
            vec![uses(&[("maximum_travel", number(20.0))])],
        ),
    );
    assert!(evaluation.findings().is_empty());
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(message.contains("a gap is too narrow"), "{message}");
}

fn metres(value: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value,
        dimension: QuantityDimension::Length,
    }
}

#[test]
fn exits_narrower_than_the_occupant_load_requires_are_found() {
    // 300 m² at 2 m² each: 150 occupants need 1.2 m.
    let run = |model: Model, geometry: Geometry| {
        evaluate(
            model,
            geometry.area("hall", 300.0, 300.0),
            with(
                exits(kind("door")),
                vec![
                    uses(&[("area_per_occupant", number(2.0))]),
                    widths(),
                    (
                        "clear_width_property",
                        property(Some("Access"), "ClearWidth"),
                    ),
                ],
            ),
        )
    };
    let evaluation = run(
        model()
            .value("d1", "Access", "ClearWidth", metres(0.9))
            .value("d2", "Access", "ClearWidth", metres(1.25)),
        Geometry::default(),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "hall".into(),
            format!(
                "exit {} is 0.9 m wide (stated clear width); 150 occupant(s) (300 m² at 2 m² \
                 each) require at least 1.2 m (use 0)",
                id("d1")
            )
        )]
    );
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");

    // Without a stated width a footprint narrower than required decides a
    // failure; a wider one decides nothing.
    let evaluation = run(
        model().value("d2", "Access", "ClearWidth", metres(1.25)),
        Geometry::default().diameter("d1", 1.0),
    );
    assert_eq!(findings(&evaluation).len(), 1, "{evaluation:?}");
    assert!(findings(&evaluation)[0].1.contains("at most 1 m wide"));
    let evaluation = run(
        model().value("d2", "Access", "ClearWidth", metres(1.25)),
        Geometry::default().diameter("d1", 1.5),
    );
    assert!(evaluation.findings().is_empty());
    assert!(
        evaluation.not_evaluated_outcomes()[0]
            .message()
            .contains("clear width of exit"),
        "{evaluation:?}"
    );
}

#[test]
fn exits_too_narrow_together_are_found() {
    // 150 occupants need 0.9 m per exit and 2 m together.
    let run = |second: f64| {
        let row: TableRow = [
            ("occupants".to_owned(), integer(200)),
            ("width".to_owned(), number(0.9)),
            ("total_width".to_owned(), number(2.0)),
        ]
        .into_iter()
        .collect();
        evaluate(
            model()
                .value("d1", "Access", "ClearWidth", metres(0.9))
                .value("d2", "Access", "ClearWidth", metres(second)),
            Geometry::default().area("hall", 300.0, 300.0),
            with(
                exits(kind("door")),
                vec![
                    uses(&[("area_per_occupant", number(2.0))]),
                    ("widths", ParameterValue::Table { value: vec![row] }),
                    (
                        "clear_width_property",
                        property(Some("Access"), "ClearWidth"),
                    ),
                ],
            ),
        )
    };
    let evaluation = run(1.0);
    assert_eq!(
        findings(&evaluation),
        [(
            "hall".into(),
            "its 2 exit(s) are at most 1.9 m wide together; 150 occupant(s) require at least 2 \
             m of exit width together (use 0)"
                .into()
        )]
    );
    let evaluation = run(1.2);
    assert!(evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty());
}

#[test]
fn an_occupant_load_across_two_rows_decides_only_widths_both_agree_on() {
    // 38 to 42 m² at 2 m² each: 19 to 21 occupants, 0.9 or 1.2 m.
    let run = |width: f64| {
        evaluate(
            model()
                .value("d1", "Access", "ClearWidth", metres(width))
                .value("d2", "Access", "ClearWidth", metres(1.5)),
            Geometry::default().area("hall", 38.0, 42.0),
            with(
                exits(kind("door")),
                vec![
                    uses(&[("area_per_occupant", number(2.0))]),
                    widths(),
                    (
                        "clear_width_property",
                        property(Some("Access"), "ClearWidth"),
                    ),
                ],
            ),
        )
    };
    let narrow = run(0.8);
    assert_eq!(findings(&narrow).len(), 1, "{narrow:?}");
    assert!(run(1.2).findings().is_empty() && unevaluated(&run(1.2)).is_empty());
    let evaluation = run(1.0);
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("hall".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn travel_from_the_rooms_door_is_measured_from_every_door() {
    // Offices start at their door `d1`; exits are the doors of the hall.
    let model = || {
        Model::default()
            .object("office", "space")
            .object("hall", "space")
            .object("d1", "door")
            .object("x1", "exit")
            .edge("bounds", "d1", "office")
            .edge("bounds", "d1", "hall")
            .edge("serves", "x1", "office")
    };
    let run = |walk: Walk| {
        model().evaluate_with(
            &EscapeRoute,
            &rule(
                CAPABILITY,
                kind("space"),
                vec![
                    (
                        "uses",
                        ParameterValue::Table {
                            value: vec![
                                [
                                    ("spaces".to_owned(), selector(kind("space"))),
                                    ("maximum_travel".to_owned(), number(30.0)),
                                    ("route_start".to_owned(), string("door")),
                                ]
                                .into_iter()
                                .collect(),
                            ],
                        },
                    ),
                    ("exit_path", strings(&["serves:backward"])),
                    ("exit_selector", selector(kind("exit"))),
                    ("door_path", strings(&["bounds:backward"])),
                    ("door_selector", selector(kind("door"))),
                    ("walking_height", number(2.0)),
                    ("walking_step", number(0.02)),
                ],
            ),
            |services| {
                Geometry::default()
                    .walk("d1", "x1", walk)
                    .register(services);
            },
        )
    };
    let evaluation = run(Walk::Between(32.0, 32.5));
    let found = findings(&evaluation);
    assert_eq!(found.len(), 2, "{evaluation:?}");
    assert_eq!(
        found[0],
        (
            "hall".into(),
            "has no exit via serves to walk to; use 0 allows at most 30 m of travel".into()
        )
    );
    assert_eq!(
        found[1],
        (
            "office".into(),
            format!(
                "door {} lies between 32 and 32.5 m from the nearest exit walking; use 0 allows \
                 at most 30 m of travel",
                id("d1")
            )
        )
    );
    let evaluation = run(Walk::Between(12.0, 12.5));
    assert_eq!(findings(&evaluation).len(), 1, "{evaluation:?}");
}

#[test]
fn declarations_that_cannot_be_judged_are_refused() {
    let base = || exits(kind("door"));
    for parameters in [
        with(base(), vec![uses(&[])]),
        with(base(), vec![uses(&[("maximum_travel", number(-1.0))])]),
        with(base(), vec![uses(&[("exits", integer(0))])]),
        with(base(), vec![uses(&[("route_start", string("door"))])]),
        with(
            base(),
            vec![uses(&[
                ("maximum_travel", number(30.0)),
                ("route_start", string("door")),
            ])],
        ),
        with(
            base(),
            vec![uses(&[
                ("maximum_travel", number(30.0)),
                ("route_start", string("lift")),
            ])],
        ),
        with(base(), vec![uses(&[("area_per_occupant", number(2.0))])]),
        with(base(), vec![uses(&[("exits", integer(1))]), widths()]),
        vec![
            ("exit_path", strings(&["bounds:backward"])),
            ("exit_selector", selector(kind("door"))),
            uses(&[("maximum_travel", number(30.0))]),
        ],
    ] {
        let evaluation = evaluate(model(), Geometry::default(), parameters);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)],
            "{evaluation:?}"
        );
    }
}

#[test]
fn without_the_routing_service_travel_is_not_evaluated() {
    let evaluation = model().evaluate(
        &EscapeRoute,
        &rule(
            CAPABILITY,
            kind("space"),
            with(
                exits(kind("door")),
                vec![uses(&[("maximum_travel", number(30.0))])],
            ),
        ),
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("hall".into(), NotEvaluatedReason::MissingService)]
    );
}

#[test]
fn a_space_no_use_picks_is_not_evaluated() {
    let evaluation = Model::default()
        .object("hall", "space")
        .object("store", "storage")
        .object("elsewhere", "room")
        .object("d9", "door")
        .edge("bounds", "d9", "elsewhere")
        .evaluate_with(
            &EscapeRoute,
            &rule(
                CAPABILITY,
                Selector::AnyOf {
                    operands: vec![kind("space"), kind("storage")],
                },
                with(exits(kind("door")), vec![uses(&[("exits", integer(1))])]),
            ),
            |services| {
                Geometry::default().register(services);
            },
        );
    assert_eq!(
        findings(&evaluation),
        [(
            "hall".into(),
            "has 0 exit(s) via bounds; use 0 requires at least 1".into()
        )],
        "{evaluation:?}"
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("store".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}
