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
    FarthestPointEvidence, FarthestPointOutcome, FarthestPointRequest, GeometryFidelity,
    LengthInterval, MetricPoint, MetricRouteOutcome, MetricRouteRequest, MetricRoutingError,
    MetricRoutingService, MetricRoutingServiceHandle, NearestTargetEvidence, NearestTargetOutcome,
    NearestTargetRequest, NotEvaluatedReason, ObjectBounds, PathTrace, PathTraceRequest, PlanArea,
    PlanAreaError, PlanAreaService, PlanAreaServiceHandle, PlanCentre, PlanLength, PlanRectangle,
    PlanSpan, PlanSpanError, PlanSpanService, PlanSpanServiceHandle, ProjectedDistanceEvidence,
    ProximityError, ProximityEvidence, ProximityProjection, ProximityRequest, ProximityService,
    ProximityServiceHandle, RectangleOrientation, ServiceRegistry, UnreachableRegionEvidence,
    UnreachableTargetsEvidence, VerticalExtent, VerticalExtentError, VerticalExtentService,
    VerticalExtentServiceHandle,
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

/// Areas, diagonals, rectangles, plan distances, walks and traces a test
/// declares. Walks are keyed by their start (`region` or door) and the
/// sorted names of their targets, followed by `~` and what they avoid.
/// Traces are keyed by the walk's start and the object; with none declared
/// the backend traces nothing.
#[derive(Default)]
struct Geometry {
    areas: BTreeMap<String, (f64, f64)>,
    diameters: BTreeMap<String, f64>,
    /// `(width, length, unique orientation)`.
    rectangles: BTreeMap<String, (f64, f64, bool)>,
    distances: BTreeMap<(String, String), f64>,
    walks: BTreeMap<(String, String), Walk>,
    traces: BTreeMap<(String, String), f64>,
    /// A backend that cannot walk around objects.
    plain: bool,
    overlaps: BTreeMap<(String, String), f64>,
    /// The points a walk from a start passes, instead of out and back.
    vias: BTreeMap<String, Vec<[f64; 3]>>,
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

    fn rectangle(mut self, local: &str, width: f64, length: f64, unique: bool) -> Self {
        self.rectangles
            .insert(local.into(), (width, length, unique));
        self
    }

    fn via(mut self, from: &str, points: &[[f64; 3]]) -> Self {
        self.vias.insert(from.into(), points.to_vec());
        self
    }

    fn overlap(mut self, first: &str, second: &str, square_metres: f64) -> Self {
        self.overlaps
            .insert((first.into(), second.into()), square_metres);
        self
    }

    fn distance(mut self, from: &str, to: &str, metres: f64) -> Self {
        self.distances.insert((from.into(), to.into()), metres);
        self
    }

    fn walk(mut self, from: &str, to: &str, walk: Walk) -> Self {
        self.walks.insert((from.into(), to.into()), walk);
        self
    }

    /// The walk from `from` to `to` keeping out of `around`.
    fn detour(self, from: &str, to: &str, around: &str, walk: Walk) -> Self {
        self.walk(from, &format!("{to}~{around}"), walk)
    }

    /// `metres` of the walk from `from` lie over `object`.
    fn trace(mut self, from: &str, object: &str, metres: f64) -> Self {
        self.traces.insert((from.into(), object.into()), metres);
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
            .register(ProximityServiceHandle::new(shared.clone()))
            .unwrap();
        services
            .register(MetricRoutingServiceHandle::new(shared))
            .unwrap();
    }

    fn lookup(&self, from: &ObjectId, targets: &[MetricPoint], avoided: &[ObjectId]) -> Walk {
        let mut names: Vec<&str> = targets
            .iter()
            .map(|target| target.subject().local_id.as_str())
            .collect();
        names.sort_unstable();
        let mut to = names.join(",");
        if !avoided.is_empty() {
            let avoided: Vec<&str> = avoided.iter().map(|id| id.local_id.as_str()).collect();
            to = format!("{to}~{}", avoided.join(","));
        }
        let key = (from.local_id.clone(), to);
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

    fn measure_plan_overlap(
        &self,
        first: &ObjectId,
        second: &ObjectId,
    ) -> Result<PlanArea, PlanAreaError> {
        let key = (first.local_id.clone(), second.local_id.clone());
        let area = *self
            .overlaps
            .get(&key)
            .unwrap_or_else(|| panic!("unexpected overlap {key:?}"));
        PlanArea::try_new(area, area, exact(format!("overlap:{}:{}", key.0, key.1)))
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

    fn measure_rectangle(&self, object: &ObjectId) -> Result<PlanRectangle, PlanSpanError> {
        let (width, length, unique) = *self
            .rectangles
            .get(&object.local_id)
            .unwrap_or_else(|| panic!("unexpected rectangle of {object}"));
        let mut evidence = exact(format!("rectangle:{}", object.local_id));
        evidence.exact = unique;
        PlanRectangle::try_new(
            object.clone(),
            [0.0, 0.0],
            0.0,
            [[1.0, 0.0], [0.0, 1.0]],
            0.0,
            [(width / 2.0, width / 2.0), (length / 2.0, length / 2.0)],
            if unique {
                RectangleOrientation::Unique
            } else {
                RectangleOrientation::Tied
            },
            evidence,
        )
    }
}

impl ProximityService for Geometry {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        panic!("escape routes read no bounds of {object}")
    }

    fn measure_proximity(&self, _: &ProximityRequest) -> Result<ProximityEvidence, ProximityError> {
        panic!("escape routes measure plan distances only")
    }

    fn measure_distance(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProjectedDistanceEvidence, ProximityError> {
        assert_eq!(request.projection(), ProximityProjection::Horizontal);
        let key = (
            request.subject().local_id.clone(),
            request.counterpart().local_id.clone(),
        );
        let metres = *self
            .distances
            .get(&key)
            .unwrap_or_else(|| panic!("unexpected plan distance {key:?}"));
        ProjectedDistanceEvidence::try_new(
            request.clone(),
            metres,
            metres,
            GeometryFidelity::Exact,
            exact(format!("distance:{}:{}", key.0, key.1)),
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
        match self.lookup(from, request.targets(), request.avoided()) {
            Walk::Between(lower, upper) => {
                // Every point stands at the same centre, so the walk goes
                // half its length out and back, unless it passes declared
                // points.
                let [x, y, z] = request.origin().coordinates_metres();
                let mut waypoints = vec![request.origin().clone()];
                match self.vias.get(&from.local_id) {
                    Some(points) => {
                        for point in points {
                            waypoints.push(MetricPoint::try_new(from.clone(), *point)?);
                        }
                    }
                    None => {
                        waypoints
                            .push(MetricPoint::try_new(from.clone(), [x + upper / 2.0, y, z])?);
                    }
                }
                waypoints.push(request.targets()[0].clone());
                Ok(NearestTargetOutcome::Reached(
                    NearestTargetEvidence::try_new(
                        0,
                        LengthInterval::try_new(lower, upper)?,
                        waypoints,
                        exact(format!("nearest:{}", from.local_id)),
                    )?,
                ))
            }
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

    fn avoids_objects(&self) -> bool {
        !self.plain
    }

    /// A declared length is exact; an object not declared for the walk's
    /// start is unmeasured.
    fn trace_path(&self, request: &PathTraceRequest) -> Result<PathTrace, MetricRoutingError> {
        if self.traces.is_empty() {
            return Err(MetricRoutingError::Unavailable("no walk is traced".into()));
        }
        let from = &request.waypoints()[0].subject().local_id;
        let lengths = request
            .objects()
            .iter()
            .map(|object| {
                self.traces
                    .get(&(from.clone(), object.local_id.clone()))
                    .ok_or_else(|| format!("{object} is not measured"))
                    .and_then(|metres| {
                        LengthInterval::exact(*metres).map_err(|error| error.to_string())
                    })
            })
            .collect();
        PathTrace::try_new(lengths, exact(format!("trace:{from}")))
    }

    fn farthest_point(
        &self,
        request: &FarthestPointRequest,
    ) -> Result<FarthestPointOutcome, MetricRoutingError> {
        assert!(request.profile().radius_metres() == 0.0);
        let region = request.region();
        let witness = MetricPoint::try_new(region.clone(), [19.0, 0.0, 0.0])?;
        match self.lookup(region, request.targets(), &[]) {
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
        // A factor below one, sharing without a path or a path without
        // sharing, and sections without travel.
        with(
            base(),
            vec![
                uses(&[("maximum_travel", number(30.0))]),
                sections(&[("stair", 0.5, None)]),
            ],
        ),
        with(
            base(),
            vec![
                uses(&[("maximum_travel", number(30.0))]),
                sections(&[("corridor", 2.0, Some(1))]),
                ("section_path", strings(&["opens:forward"])),
            ],
        ),
        with(
            base(),
            vec![
                uses(&[("maximum_travel", number(30.0))]),
                sections(&[("corridor", 2.0, Some(2))]),
            ],
        ),
        with(
            base(),
            vec![
                uses(&[("maximum_travel", number(30.0))]),
                sections(&[("stair", 2.0, None)]),
                ("section_path", strings(&["opens:forward"])),
            ],
        ),
        with(
            base(),
            vec![
                uses(&[("exits", integer(1))]),
                sections(&[("stair", 2.0, None)]),
            ],
        ),
        // Passages need a passage width in every row, and a passage width
        // needs passages.
        with(
            base(),
            vec![
                uses(&[("area_per_occupant", number(2.0))]),
                widths(),
                ("passage_selector", selector(kind("corridor"))),
            ],
        ),
        with(
            base(),
            vec![
                uses(&[("area_per_occupant", number(2.0))]),
                passage_widths(),
            ],
        ),
        with(
            base(),
            vec![
                uses(&[("area_per_occupant", number(2.0))]),
                widths(),
                ("passage_path", strings(&["opens:forward"])),
            ],
        ),
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
fn walked_passages_that_cannot_be_walked_are_refused() {
    // Walked passages need passages and doors to walk from, and are never
    // also declared.
    let base = || exits(kind("door"));
    for parameters in [
        with(
            base(),
            vec![
                uses(&[("area_per_occupant", number(2.0))]),
                widths(),
                ("walked_passages", ParameterValue::Boolean { value: true }),
            ],
        ),
        with(
            base(),
            vec![
                uses(&[("area_per_occupant", number(2.0))]),
                passage_widths(),
                ("passage_selector", selector(kind("corridor"))),
                ("walked_passages", ParameterValue::Boolean { value: true }),
            ],
        ),
        with(
            base(),
            vec![
                uses(&[("area_per_occupant", number(2.0))]),
                passage_widths(),
                ("passage_selector", selector(kind("corridor"))),
                ("passage_path", strings(&["opens:forward"])),
                ("door_path", strings(&["bounds:backward"])),
                ("door_selector", selector(kind("door"))),
                ("walked_passages", ParameterValue::Boolean { value: true }),
            ],
        ),
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

fn sections(rows: &[(&str, f64, Option<i64>)]) -> (&'static str, ParameterValue) {
    (
        "sections",
        ParameterValue::Table {
            value: rows
                .iter()
                .map(|(objects, factor, shared_by)| {
                    let mut row: TableRow = [
                        ("label".to_owned(), string(objects)),
                        ("objects".to_owned(), selector(kind(objects))),
                        ("factor".to_owned(), number(*factor)),
                    ]
                    .into_iter()
                    .collect();
                    if let Some(count) = shared_by {
                        row.insert("shared_by".into(), integer(*count));
                    }
                    row
                })
                .collect(),
        },
    )
}

#[test]
fn travel_on_a_stair_counts_by_its_factor_where_the_walk_may_reach_it() {
    // The plain walk is 12 to 12.1 m; on the stair it counts twice.
    let run = |maximum: f64, stair: Option<f64>| {
        let mut geometry = Geometry::default().walk("hall", "d1,d2", Walk::Between(12.0, 12.1));
        if let Some(metres) = stair {
            geometry = geometry.distance("hall", "st", metres);
        }
        evaluate(
            model().object("st", "stair"),
            geometry,
            with(
                exits(kind("door")),
                vec![
                    uses(&[("maximum_travel", number(maximum))]),
                    sections(&[("stair", 2.0, None)]),
                ],
            ),
        )
    };
    // Twice the whole walk is within 30 m: no distance is asked.
    let evaluation = run(30.0, None);
    assert!(evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty());
    // Within 20 m only if the walk cannot reach the stair: 15 m away in
    // plan it cannot.
    let evaluation = run(20.0, Some(15.0));
    assert!(evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty());
    // 5 m away it may: up to 24.2 m, so neither a pass nor a finding.
    let evaluation = run(20.0, Some(5.0));
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("hall".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(
        message.contains(
            "between 12 and 24.2 m walking, counting the walk on section 0 (stair) up to 2 times"
        ),
        "{message}"
    );
    // The plain walk already exceeds 10 m, whatever it crosses.
    let evaluation = run(10.0, None);
    assert_eq!(findings(&evaluation).len(), 1, "{evaluation:?}");
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");
}

#[test]
fn a_shared_section_multiplies_only_where_enough_spaces_reach_it() {
    // Hall and office both open onto corridor `c`, 0 m away in plan.
    let run = |factor: f64, shared_by: i64| {
        model()
            .object("office", "space")
            .object("d3", "door")
            .object("c", "corridor")
            .edge("bounds", "d3", "office")
            .edge("opens", "hall", "c")
            .edge("opens", "office", "c")
            .evaluate_with(
                &EscapeRoute,
                &rule(
                    CAPABILITY,
                    kind("space"),
                    with(
                        exits(kind("door")),
                        vec![
                            uses(&[("maximum_travel", number(20.0))]),
                            sections(&[("corridor", factor, Some(shared_by))]),
                            ("section_path", strings(&["opens:forward"])),
                        ],
                    ),
                ),
                |services| {
                    Geometry::default()
                        .walk("hall", "d1,d2", Walk::Between(12.0, 12.1))
                        .walk("office", "d3", Walk::Between(12.0, 12.1))
                        .distance("hall", "c", 0.0)
                        .distance("office", "c", 0.0)
                        .register(services);
                },
            )
    };
    // Shared by two, at 1.5 times: at most 18.15 m, within 20 m.
    let evaluation = run(1.5, 2);
    assert!(evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty());
    // At twice, up to 24.2 m: neither passes nor fails.
    let evaluation = run(2.0, 2);
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("hall".into(), NotEvaluatedReason::IncompleteEvidence),
            ("office".into(), NotEvaluatedReason::IncompleteEvidence)
        ]
    );
    // Only two spaces reach it, so a section shared by three is not one.
    let evaluation = run(2.0, 3);
    assert!(evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty());
}

/// Hall `hall` (two exits 1.25 m wide) and `office` (none) open onto
/// corridor `c`.
fn corridor_model() -> Model {
    model()
        .object("office", "space")
        .object("c", "corridor")
        .edge("opens", "hall", "c")
        .edge("opens", "office", "c")
        .value("d1", "Access", "ClearWidth", metres(1.25))
        .value("d2", "Access", "ClearWidth", metres(1.25))
}

fn passage_widths() -> (&'static str, ParameterValue) {
    let row = |occupants: i64, width: f64, passage: f64| -> TableRow {
        [
            ("occupants".to_owned(), integer(occupants)),
            ("width".to_owned(), number(width)),
            ("passage_width".to_owned(), number(passage)),
        ]
        .into_iter()
        .collect()
    };
    (
        "widths",
        ParameterValue::Table {
            value: vec![row(20, 0.9, 1.0), row(200, 1.2, 1.5)],
        },
    )
}

fn passage_parameters(uses: (&'static str, ParameterValue)) -> Vec<(&'static str, ParameterValue)> {
    with(
        exits(kind("door")),
        vec![
            uses,
            passage_widths(),
            (
                "clear_width_property",
                property(Some("Access"), "ClearWidth"),
            ),
            ("passage_path", strings(&["opens:forward"])),
            ("passage_selector", selector(kind("corridor"))),
            (
                "passage_width_property",
                property(Some("Corridor"), "ClearWidth"),
            ),
        ],
    )
}

/// 300 m² of hall and 40 m² of office at 2 m² each.
fn passages(model: Model, geometry: Geometry) -> CapabilityEvaluation {
    evaluate(
        model,
        geometry
            .area("hall", 300.0, 300.0)
            .area("office", 40.0, 40.0),
        passage_parameters(uses(&[("area_per_occupant", number(2.0))])),
    )
}

#[test]
fn a_passage_narrower_than_all_its_occupants_require_is_found() {
    // 150 occupants from the hall and 20 from the office: 170 need 1.5 m,
    // though the office's 20 alone would need only 1 m.
    let evaluation = passages(
        corridor_model().value("c", "Corridor", "ClearWidth", metres(1.2)),
        Geometry::default(),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "c".into(),
            format!(
                "passage {} is 1.2 m wide (stated clear width); 170 occupant(s) relying on it \
                 (from {}, {}) require at least 1.5 m",
                id("c"),
                id("hall"),
                id("office")
            )
        )]
    );
    assert_eq!(
        evaluation.findings()[0].related,
        vec![id("hall"), id("office")]
    );
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");
    let evaluation = passages(
        corridor_model().value("c", "Corridor", "ClearWidth", metres(1.6)),
        Geometry::default(),
    );
    assert!(evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty());
}

#[test]
fn without_a_stated_width_the_enclosing_rectangle_decides_only_a_failure() {
    let evaluation = passages(
        corridor_model(),
        Geometry::default().rectangle("c", 1.2, 30.0, true),
    );
    assert_eq!(findings(&evaluation).len(), 1, "{evaluation:?}");
    assert!(
        findings(&evaluation)[0]
            .1
            .contains("is at most 1.2 m wide (the shorter side of the rectangle"),
        "{evaluation:?}"
    );
    let evaluation = passages(
        corridor_model(),
        Geometry::default().rectangle("c", 2.0, 30.0, true),
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("c".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    // A tied orientation has no known sides, so nothing is decided.
    let evaluation = passages(
        corridor_model(),
        Geometry::default().rectangle("c", 1.2, 30.0, false),
    );
    assert!(evaluation.findings().is_empty());
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(message.contains("several orientations"), "{message}");
}

#[test]
fn a_passage_serving_a_space_of_unknown_load_is_not_evaluated() {
    // The office's use states no area per occupant, so it may bring any
    // number of occupants: even a 3 m corridor is not passed.
    let row = |spaces: &str, cells: &[(&str, ParameterValue)]| -> TableRow {
        let mut row: TableRow = cells
            .iter()
            .map(|(column, value)| ((*column).to_owned(), value.clone()))
            .collect();
        row.insert("spaces".into(), selector(kind(spaces)));
        row
    };
    let evaluation = Model::default()
        .object("hall", "space")
        .object("d1", "door")
        .object("office", "office")
        .object("c", "corridor")
        .edge("bounds", "d1", "hall")
        .edge("opens", "hall", "c")
        .edge("opens", "office", "c")
        .value("c", "Corridor", "ClearWidth", metres(3.0))
        .value("d1", "Access", "ClearWidth", metres(1.25))
        .evaluate_with(
            &EscapeRoute,
            &rule(
                CAPABILITY,
                Selector::AnyOf {
                    operands: vec![kind("space"), kind("office")],
                },
                passage_parameters((
                    "uses",
                    ParameterValue::Table {
                        value: vec![
                            row("space", &[("area_per_occupant", number(2.0))]),
                            row("office", &[("exits", integer(1))]),
                        ],
                    },
                )),
            ),
            |services| {
                Geometry::default()
                    .area("hall", 30.0, 30.0)
                    .register(services);
            },
        );
    assert_eq!(
        findings(&evaluation),
        [(
            "office".into(),
            "has 0 exit(s) via bounds; use 1 requires at least 1".into()
        )]
    );
    let outcome = evaluation
        .not_evaluated_outcomes()
        .iter()
        .find(|outcome| outcome.object_id() == Some(&id("c")))
        .expect("the corridor is not evaluated");
    assert!(
        outcome.message().contains(&format!(
            "the occupants relying on passage {} are unknown: {}: its use states no \
             `area_per_occupant`",
            id("c"),
            id("office")
        )),
        "{}",
        outcome.message()
    );
}

/// Exit doors must open out of the space: `d1` swings into the hall,
/// `d2` out of it, `o1` is an opening without leaves, `d3` slides.
#[test]
fn an_exit_door_opening_into_the_space_is_found() {
    use common::doors::{Doors, Rooms, hinged, sliding};
    let east = [1.0, 0.0, 0.0];
    let with_exits = || {
        model()
            .object("o1", "door")
            .object("d3", "door")
            .edge("bounds", "o1", "hall")
            .edge("bounds", "d3", "hall")
    };
    let run = |model: Model, sliding_exit: bool| {
        let mut doors = Doors::default()
            .door(
                "d1",
                vec![hinged([0.0; 3], east, [0.0, 1.0, 0.0], 0.9, false)],
                1.0,
                None,
            )
            .door(
                "d2",
                vec![hinged([3.0, 0.0, 0.0], east, [0.0, -1.0, 0.0], 0.9, false)],
                1.0,
                None,
            );
        if sliding_exit {
            doors = doors.door("d3", vec![sliding([6.0, 0.0, 0.0], east, 0.9)], 1.0, None);
        }
        let parameters = with(
            exits(kind("door")),
            vec![
                uses(&[("exits", integer(1))]),
                ("exit_door_direction", common::boolean(true)),
            ],
        );
        model.evaluate_with(
            &EscapeRoute,
            &rule(CAPABILITY, kind("space"), parameters),
            |services| {
                services.register(doors.handle()).unwrap();
                services
                    .register(
                        Rooms::default()
                            .room("hall", [-5.0, 0.0], [10.0, 4.0])
                            .handle(),
                    )
                    .unwrap();
            },
        )
    };
    let evaluation = run(with_exits(), true);
    assert_eq!(
        findings(&evaluation),
        [(
            "hall".into(),
            "exit door test:model/d1 opens into the space, against the direction of escape".into()
        )]
    );
    let [(space, reason)] = &unevaluated(&evaluation)[..] else {
        panic!("{:?}", unevaluated(&evaluation))
    };
    assert_eq!(
        (space.as_str(), reason),
        ("hall", &NotEvaluatedReason::IncompleteEvidence)
    );
    assert!(
        evaluation.not_evaluated_outcomes()[0]
            .message()
            .contains("d3 has no hinged leaf"),
        "{:?}",
        evaluation.not_evaluated_outcomes()
    );
    // Without the sliding exit, only the opening is left, and it has no leaf.
    let evaluation = run(
        model().object("o1", "door").edge("bounds", "o1", "hall"),
        false,
    );
    assert_eq!(findings(&evaluation).len(), 1);
    assert!(
        unevaluated(&evaluation).is_empty(),
        "{:?}",
        unevaluated(&evaluation)
    );
}

/// Office `office` and hall `hall` leave by their doors `d1` and `d2`
/// towards exit `x1`, 2 m wide; corridor `c` is 1.2 m wide.
fn office_and_hall() -> Model {
    Model::default()
        .object("office", "space")
        .object("hall", "space")
        .object("d1", "door")
        .object("d2", "door")
        .object("x1", "exit")
        .object("c", "corridor")
        .edge("bounds", "d1", "office")
        .edge("bounds", "d2", "hall")
        .edge("serves", "x1", "office")
        .edge("serves", "x1", "hall")
        .value("c", "Corridor", "ClearWidth", metres(1.2))
        .value("x1", "Access", "ClearWidth", metres(2.0))
}

fn doors_and_exits() -> Vec<(&'static str, ParameterValue)> {
    vec![
        (
            "clear_width_property",
            property(Some("Access"), "ClearWidth"),
        ),
        ("exit_path", strings(&["serves:backward"])),
        ("exit_selector", selector(kind("exit"))),
        ("door_path", strings(&["bounds:backward"])),
        ("door_selector", selector(kind("door"))),
        ("walking_height", number(2.0)),
        ("walking_step", number(0.02)),
    ]
}

#[test]
fn a_door_walk_counts_the_metres_it_walks_on_a_section_by_its_factor() {
    // The walk from `d1` is 12 to 12.1 m, and the stair lies 5 m from the
    // door, so it may be crossed: without a trace up to 24.2 m. The hall's
    // 5 m walk is within 20 m at any factor.
    let run = |on_stair: Option<f64>| {
        let mut geometry = Geometry::default()
            .walk("d1", "x1", Walk::Between(12.0, 12.1))
            .walk("d2", "x1", Walk::Between(5.0, 5.0))
            .distance("d1", "st", 5.0);
        if let Some(metres) = on_stair {
            geometry = geometry.trace("d1", "st", metres);
        }
        office_and_hall().object("st", "stair").evaluate_with(
            &EscapeRoute,
            &rule(
                CAPABILITY,
                kind("space"),
                with(
                    doors_and_exits(),
                    vec![
                        (
                            "uses",
                            ParameterValue::Table {
                                value: vec![
                                    [
                                        ("spaces".to_owned(), selector(kind("space"))),
                                        ("maximum_travel".to_owned(), number(20.0)),
                                        ("route_start".to_owned(), string("door")),
                                    ]
                                    .into_iter()
                                    .collect(),
                                ],
                            },
                        ),
                        sections(&[("stair", 2.0, None)]),
                    ],
                ),
            ),
            |services| geometry.register(services),
        )
    };
    // 3 m of the walk on the stair: at most 12.1 + 3 = 15.1 m, within 20 m.
    let evaluation = run(Some(3.0));
    assert!(
        evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty(),
        "{evaluation:?}"
    );
    // 9 m on it: at most 21.1 m, which neither passes nor fails.
    let evaluation = run(Some(9.0));
    assert!(evaluation.findings().is_empty());
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(
        message.contains(
            "between 12 and 21.1 m walking, counting the walk on section 0 (stair) up to 2 times"
        ),
        "{message}"
    );
    // The walk not on the stair at all counts plain.
    let evaluation = run(Some(0.0));
    assert!(evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty());
    // A backend that cannot trace leaves today's bound: up to 24.2 m.
    let evaluation = run(None);
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(
        message.contains("between 12 and 24.2 m walking"),
        "{message}"
    );
}

/// 40 m² of office (20 occupants) and 300 m² of hall (150) at 2 m² each;
/// the corridor needs 1 m for up to 20 occupants and 1.5 m for up to 200.
fn walked(geometry: Geometry) -> CapabilityEvaluation {
    walked_with(office_and_hall(), geometry)
}

fn walked_with(model: Model, geometry: Geometry) -> CapabilityEvaluation {
    model.evaluate_with(
        &EscapeRoute,
        &rule(
            CAPABILITY,
            kind("space"),
            with(
                doors_and_exits(),
                vec![
                    uses(&[("area_per_occupant", number(2.0))]),
                    passage_widths(),
                    ("passage_selector", selector(kind("corridor"))),
                    (
                        "passage_width_property",
                        property(Some("Corridor"), "ClearWidth"),
                    ),
                    ("walked_passages", ParameterValue::Boolean { value: true }),
                ],
            ),
        ),
        |services| {
            geometry
                .area("office", 40.0, 40.0)
                .area("hall", 300.0, 300.0)
                .register(services);
        },
    )
}

/// The office's walk crosses the corridor, and no walk round it reaches
/// the exit.
fn office_through_the_corridor() -> Geometry {
    Geometry::default()
        .walk("d1", "x1", Walk::Between(10.0, 10.0))
        .trace("d1", "c", 6.0)
        .detour("d1", "x1", "c", Walk::Unreachable)
        .walk("d2", "x1", Walk::Between(8.0, 8.0))
}

#[test]
fn a_passage_every_shortest_walk_crosses_carries_its_occupants() {
    // The hall's walk keeps off the corridor (its door lies 5 m from it
    // and it 4 m from the exit, more than the hall's 8 m walk), so only the
    // office's 20 occupants rely on it: 1 m is enough.
    let evaluation = walked(
        office_through_the_corridor()
            .trace("d2", "c", 0.0)
            .distance("d2", "c", 5.0)
            .distance("c", "x1", 4.0),
    );
    assert!(
        evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty(),
        "{evaluation:?}"
    );

    // The hall's walk crosses it too, and the walk round it is 9 m or
    // more, longer than the hall's 8 m: every shortest walk crosses it.
    let evaluation = walked(office_through_the_corridor().trace("d2", "c", 3.0).detour(
        "d2",
        "x1",
        "c",
        Walk::Between(9.0, 9.5),
    ));
    assert_eq!(
        findings(&evaluation),
        [(
            "c".into(),
            format!(
                "passage {} is 1.2 m wide (stated clear width); 170 occupant(s) relying on it \
                 (from {}, {}) require at least 1.5 m",
                id("c"),
                id("hall"),
                id("office")
            )
        )]
    );
    let finding = &evaluation.findings()[0];
    assert_eq!(finding.related, vec![id("hall"), id("office")]);
    for locator in ["nearest:d1", "cut-off:d1", "nearest:d2"] {
        assert!(
            finding.evidence.iter().any(|item| item.locator == locator),
            "{locator}: {finding:?}"
        );
    }
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");
}

#[test]
fn a_passage_on_only_one_of_two_shortest_walks_is_not_relied_on_surely() {
    // The hall's witness walk crosses the corridor, but a walk round it is
    // just as short: its occupants may or may not rely on it, so 20 to 170
    // occupants need 1 to 1.5 m, and 1.2 m decides nothing.
    let evaluation = walked(office_through_the_corridor().trace("d2", "c", 3.0).detour(
        "d2",
        "x1",
        "c",
        Walk::Between(8.0, 8.0),
    ));
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    assert_eq!(
        unevaluated(&evaluation),
        [("c".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(
        message.contains(&format!(
            "between 20 and 170 occupants relying on it (from {}; perhaps also from {})",
            id("office"),
            id("hall")
        )),
        "{message}"
    );
}

#[test]
fn a_passage_no_walk_surely_crosses_is_never_found_too_narrow() {
    // Whether a walk round the corridor exists is unknown: with every
    // detour refused, both spaces only perhaps rely on it, and a 0.8 m
    // corridor, too narrow for either load, is still no finding.
    let evaluation = walked_with(
        office_and_hall().value("c", "Corridor", "ClearWidth", metres(0.8)),
        Geometry::default()
            .walk("d1", "x1", Walk::Between(10.0, 10.0))
            .walk("d2", "x1", Walk::Between(8.0, 8.0))
            .trace("d1", "c", 6.0)
            .trace("d2", "c", 3.0)
            .detour("d1", "x1", "c", Walk::Refused)
            .detour("d2", "x1", "c", Walk::Refused),
    );
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(
        message.contains("no walk surely crosses it")
            && message.contains(&format!("perhaps from {}, {}", id("hall"), id("office"))),
        "{message}"
    );
}

/// Hall `hall` leaves by its door `d1` towards exits `x1` and `x2`; door
/// `ld`, somewhere on the way, is locked where the model says so.
fn hall_with_a_locked_door() -> Model {
    Model::default()
        .object("hall", "space")
        .object("d1", "door")
        .object("ld", "door")
        .object("x1", "exit")
        .object("x2", "exit")
        .edge("bounds", "d1", "hall")
        .edge("serves", "x1", "hall")
        .edge("serves", "x2", "hall")
}

fn exists(set: &str, name: &str) -> Selector {
    Selector::Property {
        property_set: Some(set.into()),
        property: name.into(),
        operator: ComparisonOperator::Exists,
        value: None,
        case_sensitive: true,
        trim: false,
        quantifier: None,
        precision: None,
    }
}

fn from_the_door(cells: &[(&str, ParameterValue)]) -> (&'static str, ParameterValue) {
    let mut row = use_row(cells);
    row.insert("route_start".into(), string("door"));
    ("uses", ParameterValue::Table { value: vec![row] })
}

#[test]
fn a_door_not_usable_for_escape_forces_the_longer_walk() {
    let run = |model: Model, geometry: Geometry, extra: Vec<(&'static str, ParameterValue)>| {
        let mut parameters = with(
            doors_and_exits(),
            vec![from_the_door(&[
                ("maximum_travel", number(20.0)),
                ("exits", integer(2)),
            ])],
        );
        parameters.extend(extra);
        model.evaluate_with(
            &EscapeRoute,
            &rule(CAPABILITY, kind("space"), parameters),
            |services| geometry.register(services),
        )
    };
    let no_escape = || vec![("no_escape_selector", selector(exists("Escape", "Locked")))];
    // Through the locked door the walk is 8 m; around it 25 m.
    let geometry = || {
        Geometry::default()
            .walk("d1", "x1,x2", Walk::Between(8.0, 8.0))
            .walk("d1", "x1,x2~ld", Walk::Between(25.0, 25.0))
    };
    let locked =
        || hall_with_a_locked_door().value("ld", "Escape", "Locked", PropertyValue::Boolean(true));
    let evaluation = run(locked(), geometry(), Vec::new());
    assert!(
        evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty(),
        "{evaluation:?}"
    );
    let evaluation = run(locked(), geometry(), no_escape());
    assert_eq!(
        findings(&evaluation),
        [(
            "hall".into(),
            format!(
                "door {} lies 25 m from the nearest exit walking; use 0 allows at most 20 m of \
                 travel",
                id("d1")
            )
        )]
    );
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");

    // An exit marked as not usable is no exit: one is left of two.
    let evaluation = run(
        locked().value("x2", "Escape", "Locked", PropertyValue::Boolean(true)),
        Geometry::default().walk("d1", "x1~ld,x2", Walk::Between(12.0, 12.0)),
        no_escape(),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "hall".into(),
            "has 1 exit(s) via serves; use 0 requires at least 2".into()
        )]
    );

    // Whether the door is locked cannot be read: the walk lies between 8
    // and 25 m, which decides nothing.
    let evaluation = run(
        hall_with_a_locked_door().unreadable("ld"),
        geometry(),
        no_escape(),
    );
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(message.contains("between 8 and 25 m walking"), "{message}");

    // A backend that cannot walk around the door answers nothing.
    let mut plain = geometry();
    plain.plain = true;
    let evaluation = run(locked(), plain, no_escape());
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(
        message.contains("this backend does not walk around objects"),
        "{message}"
    );
}

#[test]
fn the_farthest_point_stands_only_where_no_walk_reaches_what_it_avoids() {
    // The farthest point is measured on the plain walk: 12 m at most. A
    // locked door 50 m away is out of reach of any such walk; one 5 m away
    // may be on it, and the travel is then unknown.
    let run = |apart: f64| {
        hall_with_a_locked_door()
            .value("ld", "Escape", "Locked", PropertyValue::Boolean(true))
            .evaluate_with(
                &EscapeRoute,
                &rule(
                    CAPABILITY,
                    kind("space"),
                    with(
                        doors_and_exits(),
                        vec![
                            uses(&[("maximum_travel", number(20.0))]),
                            ("no_escape_selector", selector(exists("Escape", "Locked"))),
                        ],
                    ),
                ),
                |services| {
                    Geometry::default()
                        .walk("hall", "x1,x2", Walk::Between(11.9, 12.0))
                        .distance("hall", "ld", apart)
                        .register(services);
                },
            )
    };
    let evaluation = run(50.0);
    assert!(
        evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty(),
        "{evaluation:?}"
    );
    let evaluation = run(5.0);
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(
        message.contains("measured on the plain walk only")
            && message.contains("at least 11.9 m walking"),
        "{message}"
    );
}

/// Room `r` opens through `d1` onto corridor `c`, both in compartment `A`;
/// fire door `fd` leads from the corridor to stair `s` in compartment `B`.
/// The building exit `x1` serves the room.
fn compartments() -> Model {
    Model::default()
        .object("r", "room")
        .object("c", "corridor")
        .object("s", "stair")
        .object("d1", "door")
        .object("fd", "door")
        .object("x1", "exit")
        .object("A", "compartment")
        .object("B", "compartment")
        .edge("bounds", "d1", "r")
        .edge("bounds", "d1", "c")
        .edge("bounds", "fd", "c")
        .edge("bounds", "fd", "s")
        .edge("serves", "x1", "r")
        .edge("in", "r", "A")
        .edge("in", "c", "A")
        .edge("in", "s", "B")
}

fn in_compartments(
    model: Model,
    geometry: Geometry,
    start: &str,
    extra: Vec<(&'static str, ParameterValue)>,
) -> CapabilityEvaluation {
    let mut row = use_row(&[("maximum_travel", number(30.0))]);
    row.insert("spaces".into(), selector(kind("room")));
    row.insert("route_start".into(), string(start));
    let mut parameters = with(
        doors_and_exits(),
        vec![("uses", ParameterValue::Table { value: vec![row] })],
    );
    parameters.extend(extra);
    model.evaluate_with(
        &EscapeRoute,
        &rule(CAPABILITY, kind("room"), parameters),
        |services| geometry.register(services),
    )
}

fn by_path() -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("compartment_selector", selector(kind("compartment"))),
        ("compartment_path", strings(&["in:forward"])),
    ]
}

#[test]
fn travel_ends_at_the_door_out_of_the_compartment() {
    // The building exit is 40 m away, the fire door out of the room's
    // compartment 18 m: travel ends there.
    let geometry = || {
        Geometry::default()
            .walk("r", "x1", Walk::Between(40.0, 40.0))
            .walk("r", "fd,x1", Walk::Between(18.0, 18.0))
    };
    let evaluation = in_compartments(compartments(), geometry(), "farthest-point", Vec::new());
    assert_eq!(
        findings(&evaluation),
        [(
            "r".into(),
            "its farthest point, around (19.00, 0.00), lies 40 m from the nearest exit walking; \
             use 0 allows at most 30 m of travel"
                .into()
        )]
    );
    let evaluation = in_compartments(compartments(), geometry(), "farthest-point", by_path());
    assert!(
        evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty(),
        "{evaluation:?}"
    );

    // Compartments by footprint overlap: the room and the corridor lie in
    // `A` (90 % and all of their footprints), the stair in `B`.
    let evaluation = in_compartments(
        compartments(),
        geometry()
            .area("r", 20.0, 20.0)
            .area("c", 30.0, 30.0)
            .area("s", 10.0, 10.0)
            .overlap("r", "A", 18.0)
            .overlap("r", "B", 0.0)
            .overlap("c", "A", 30.0)
            .overlap("c", "B", 0.0)
            .overlap("s", "A", 0.0)
            .overlap("s", "B", 10.0),
        "farthest-point",
        vec![
            ("compartment_selector", selector(kind("compartment"))),
            ("compartment_overlap", number(0.8)),
        ],
    );
    assert!(
        evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty(),
        "{evaluation:?}"
    );

    // Whether the stair lies in `A` is unknown (76 to 84 %): the fire door
    // may lead out or not, so the travel lies between 18 and 40 m.
    let evaluation = in_compartments(
        compartments(),
        geometry()
            .area("r", 20.0, 20.0)
            .area("c", 30.0, 30.0)
            .area("s", 9.5, 10.5)
            .overlap("r", "A", 18.0)
            .overlap("r", "B", 0.0)
            .overlap("c", "A", 30.0)
            .overlap("c", "B", 0.0)
            .overlap("s", "A", 8.0)
            .overlap("s", "B", 2.0),
        "farthest-point",
        vec![
            ("compartment_selector", selector(kind("compartment"))),
            ("compartment_overlap", number(0.8)),
        ],
    );
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(
        message.contains(&format!(
            "the longest travel to the nearest exit or door out of compartment {} is between \
             18 and 40 m walking",
            id("A")
        )),
        "{message}"
    );
}

#[test]
fn a_walk_through_a_higher_ranked_zone_is_excluded() {
    // Hazard `h` ranks above the room: the walk from the door keeps out of
    // it and is 35 m long, where the walk through it is 10 m.
    let geometry = || {
        Geometry::default()
            .walk("d1", "fd,x1", Walk::Between(10.0, 10.0))
            .walk("d1", "fd,x1~h", Walk::Between(35.0, 35.0))
    };
    let zones = || {
        let row = |objects: &str, rank: i64| -> TableRow {
            [
                ("objects".to_owned(), selector(kind(objects))),
                ("rank".to_owned(), integer(rank)),
            ]
            .into_iter()
            .collect()
        };
        (
            "zones",
            ParameterValue::Table {
                value: vec![row("room", 1), row("hazard", 2)],
            },
        )
    };
    let model = || compartments().object("h", "hazard");
    let evaluation = in_compartments(model(), geometry(), "door", by_path());
    assert!(
        evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty(),
        "{evaluation:?}"
    );
    let evaluation = in_compartments(model(), geometry(), "door", with(by_path(), vec![zones()]));
    assert_eq!(
        findings(&evaluation),
        [(
            "r".into(),
            format!(
                "door {} lies 35 m from the nearest exit or door out of compartment {} walking; \
                 use 0 allows at most 30 m of travel",
                id("d1"),
                id("A")
            )
        )]
    );
    // A room no row ranks cannot tell what ranks above it.
    let evaluation = in_compartments(
        compartments().object("h", "hazard").object("r2", "room"),
        geometry(),
        "door",
        with(
            by_path(),
            vec![(
                "zones",
                ParameterValue::Table {
                    value: vec![
                        [
                            ("objects".to_owned(), selector(kind("hazard"))),
                            ("rank".to_owned(), integer(2)),
                        ]
                        .into_iter()
                        .collect(),
                    ],
                },
            )],
        ),
    );
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(message.contains("no row of `zones` ranks it"), "{message}");
}

#[test]
fn a_model_without_compartments_is_inadequate_information() {
    let model = Model::default()
        .object("r", "room")
        .object("d1", "door")
        .object("x1", "exit")
        .edge("bounds", "d1", "r")
        .edge("serves", "x1", "r");
    let evaluation = in_compartments(model, Geometry::default(), "farthest-point", by_path());
    assert_eq!(
        findings(&evaluation),
        [(
            "source".into(),
            "inadequate information: no compartment is modelled (`compartment_selector` picks \
             nothing), so where escape travel ends at a compartment boundary is unknown"
                .into()
        )]
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("r".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert!(
        evaluation.not_evaluated_outcomes()[0]
            .message()
            .contains("it lies in no compartment")
    );
}

/// Room `r` reaches exits `x1` and `x2`, both at the end of corridor `c`.
fn routes(geometry: Geometry) -> CapabilityEvaluation {
    Model::default()
        .object("r", "space")
        .object("c", "corridor")
        .object("x1", "exit")
        .object("x2", "exit")
        .edge("serves", "x1", "r")
        .edge("serves", "x2", "r")
        .evaluate_with(
            &EscapeRoute,
            &rule(
                CAPABILITY,
                kind("space"),
                with(
                    doors_and_exits(),
                    vec![
                        uses(&[("exits", integer(2))]),
                        ("exit_count", string("routes")),
                        ("passage_selector", selector(kind("corridor"))),
                    ],
                ),
            ),
            |services| geometry.register(services),
        )
}

fn two_exits() -> Geometry {
    Geometry::default()
        .walk("r", "x1", Walk::Between(12.0, 12.0))
        .walk("r", "x2", Walk::Between(14.0, 14.0))
        .walk("r", "x1,x2", Walk::Between(12.0, 12.0))
}

#[test]
fn two_exits_through_one_dead_end_corridor_count_as_one_route() {
    // Every walk crosses the corridor: without it no exit is reached.
    let evaluation = routes(two_exits().trace("r", "c", 6.0).detour(
        "r",
        "x1,x2",
        "c",
        Walk::Unreachable,
    ));
    assert_eq!(
        findings(&evaluation),
        [(
            "r".into(),
            format!(
                "every walk from it to an exit passes through {}, so it has at most 1 \
                 independent route(s); use 0 requires at least 2",
                id("c")
            )
        )]
    );
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");
    let finding = &evaluation.findings()[0];
    assert!(
        finding
            .evidence
            .iter()
            .any(|item| item.locator == "cut-off:r"),
        "{finding:?}"
    );

    // Walks keeping off the corridor are two independent routes.
    let evaluation = routes(two_exits().trace("r", "c", 0.0));
    assert!(
        evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty(),
        "{evaluation:?}"
    );

    // Both walks cross it, but a longer walk round it exists: one to two
    // routes, which decides nothing.
    let evaluation = routes(two_exits().trace("r", "c", 6.0).detour(
        "r",
        "x1,x2",
        "c",
        Walk::Between(30.0, 30.0),
    ));
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(
        message.contains("it has at least 1 and at most 2 independent route(s)"),
        "{message}"
    );
}

/// Hall `hall` is left by its door `d1` towards exit `x1` through corridor
/// door `cd`: the walk from `d1` runs east through `cd`, whose leaf closes
/// along x = 3, and comes back far north of it.
fn through_a_corridor_door() -> Model {
    Model::default()
        .object("hall", "space")
        .object("d1", "door")
        .object("cd", "door")
        .object("x1", "exit")
        .edge("bounds", "d1", "hall")
        .edge("serves", "x1", "hall")
        .value("hall", "Access", "ClearHeight", metres(3.0))
        .value("d1", "Access", "ClearHeight", metres(2.1))
}

fn along_the_route(
    model: Model,
    geometry: Geometry,
    corridor_door_opens: [f64; 3],
    maximum: f64,
    extra: Vec<(&'static str, ParameterValue)>,
) -> CapabilityEvaluation {
    use common::doors::{Doors, Rooms, hinged};
    let doors = Doors::default()
        .door(
            "d1",
            vec![hinged(
                [3.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, -1.0, 0.0],
                0.9,
                false,
            )],
            1.0,
            None,
        )
        .door(
            "cd",
            vec![hinged(
                [3.0, 1.5, 0.0],
                [0.0, 1.0, 0.0],
                corridor_door_opens,
                1.0,
                false,
            )],
            1.0,
            None,
        );
    let rooms = Rooms::default().room("hall", [-5.0, 0.0], [10.0, 4.0]);
    let mut parameters = with(
        doors_and_exits(),
        vec![from_the_door(&[("maximum_travel", number(maximum))])],
    );
    parameters.extend(extra);
    model.evaluate_with(
        &EscapeRoute,
        &rule(CAPABILITY, kind("space"), parameters),
        |services| {
            geometry
                .via("d1", &[[5.0, 2.0, 0.0], [5.0, 10.0, 0.0], [1.0, 10.0, 0.0]])
                .register(services);
            services.register(doors.handle()).unwrap();
            services.register(rooms.handle()).unwrap();
        },
    )
}

fn walk_through_the_corridor_door(around: Walk) -> Geometry {
    Geometry::default()
        .walk("d1", "x1", Walk::Between(10.0, 10.0))
        .trace("d1", "cd", 1.0)
        .detour("d1", "x1", "cd", around)
}

#[test]
fn a_corridor_door_swinging_against_the_route_is_found() {
    let east = [1.0, 0.0, 0.0];
    let west = [-1.0, 0.0, 0.0];
    let direction = || vec![("route_door_direction", common::boolean(true))];
    let evaluation = along_the_route(
        through_a_corridor_door(),
        walk_through_the_corridor_door(Walk::Unreachable),
        east,
        20.0,
        direction(),
    );
    assert!(
        evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty(),
        "{evaluation:?}"
    );
    let evaluation = along_the_route(
        through_a_corridor_door(),
        walk_through_the_corridor_door(Walk::Unreachable),
        west,
        20.0,
        direction(),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "hall".into(),
            format!(
                "door {} on its route opens against the direction of escape",
                id("cd")
            )
        )]
    );
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");
    // A walk round the door as short as the named one: another shortest
    // walk may not cross it, so nothing is found.
    let evaluation = along_the_route(
        through_a_corridor_door(),
        walk_through_the_corridor_door(Walk::Between(10.0, 10.0)),
        west,
        20.0,
        direction(),
    );
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(
        message.contains("not every shortest walk is proven to cross it"),
        "{message}"
    );
}

#[test]
fn a_door_lower_than_the_minimum_is_found_while_the_travel_is_judged() {
    let heights = || {
        vec![
            ("minimum_clear_height", number(2.1)),
            (
                "clear_height_property",
                property(Some("Access"), "ClearHeight"),
            ),
        ]
    };
    // The 10 m walk exceeds 8 m, and the corridor door is 1.9 m high.
    let evaluation = along_the_route(
        through_a_corridor_door().value("cd", "Access", "ClearHeight", metres(1.9)),
        walk_through_the_corridor_door(Walk::Unreachable),
        [1.0, 0.0, 0.0],
        8.0,
        heights(),
    );
    let found = findings(&evaluation);
    assert_eq!(found.len(), 2, "{evaluation:?}");
    assert_eq!(
        found[0],
        (
            "hall".into(),
            format!(
                "door {} lies 10 m from the nearest exit walking; use 0 allows at most 8 m of \
                 travel",
                id("d1")
            )
        )
    );
    assert!(
        found[1].1.starts_with(&format!(
            "{} on its route is 1.9 m high (clear height (",
            id("cd")
        )) && found[1]
            .1
            .ends_with("); its route needs at least 2.1 m of clear height"),
        "{found:?}"
    );
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");
    // Without a stated height, the door's 2.1 m vertical extent decides
    // nothing.
    let evaluation = along_the_route(
        through_a_corridor_door(),
        walk_through_the_corridor_door(Walk::Unreachable),
        [1.0, 0.0, 0.0],
        20.0,
        heights(),
    );
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(
        message.contains(&format!(
            "the clear height of {} on its route is not stated",
            id("cd")
        )),
        "{message}"
    );
}

#[test]
fn a_shared_stretch_counts_by_the_common_path_factor() {
    // The walks from `d1` to either exit cross corridor `c` for `shared`
    // metres before they part; the nearer exit is 15 m away.
    let run = |shared: f64, factor: Option<f64>| {
        let mut parameters = with(
            doors_and_exits(),
            vec![from_the_door(&[("maximum_travel", number(20.0))])],
        );
        if let Some(factor) = factor {
            parameters.push(("common_path_factor", number(factor)));
            parameters.push(("passage_selector", selector(kind("corridor"))));
        }
        hall_with_a_locked_door()
            .object("c", "corridor")
            .evaluate_with(
                &EscapeRoute,
                &rule(CAPABILITY, kind("space"), parameters),
                |services| {
                    Geometry::default()
                        .walk("d1", "x1,x2", Walk::Between(15.0, 15.0))
                        .walk("d1", "x2", Walk::Between(18.0, 18.0))
                        .trace("d1", "c", shared)
                        .register(services);
                },
            )
    };
    // 10 m shared count twice: up to 25 m, which decides nothing.
    let evaluation = run(10.0, Some(2.0));
    assert!(evaluation.findings().is_empty(), "{evaluation:?}");
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(
        message.contains(
            "the longest travel to the nearest exit is between 15 and 25 m walking, its common \
             path counting 2 times"
        ),
        "{message}"
    );
    // 3 m shared: at most 18 m.
    let evaluation = run(3.0, Some(2.0));
    assert!(
        evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty(),
        "{evaluation:?}"
    );
    // Without the factor, the walk counts plain.
    let evaluation = run(10.0, None);
    assert!(
        evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty(),
        "{evaluation:?}"
    );
}

/// Rooms `r1` to `r3` (40 m² each) leave by their doors `d1` to `d3`
/// towards exit `x1`, 2 m wide, through corridor door `cd`.
fn three_rooms_through_one_door() -> Model {
    let mut model = Model::default()
        .object("x1", "exit")
        .object("cd", "corridor door")
        .value("x1", "Access", "ClearWidth", metres(2.0));
    for room in 1..=3 {
        let (space, door) = (format!("r{room}"), format!("d{room}"));
        model = model
            .object(&space, "space")
            .object(&door, "door")
            .edge("bounds", &door, &space)
            .edge("serves", "x1", &space);
    }
    model
}

fn door_widths(rows: &[(i64, f64, &str, f64)]) -> (&'static str, ParameterValue) {
    (
        "widths",
        ParameterValue::Table {
            value: rows
                .iter()
                .map(|(occupants, width, column, value)| {
                    [
                        ("occupants".to_owned(), integer(*occupants)),
                        ("width".to_owned(), number(*width)),
                        ((*column).to_owned(), number(*value)),
                    ]
                    .into_iter()
                    .collect()
                })
                .collect(),
        },
    )
}

#[test]
fn a_corridor_door_carrying_three_rooms_is_as_wide_as_all_their_occupants_need() {
    let run = |width: f64| {
        let mut geometry = Geometry::default();
        for room in 1..=3 {
            let door = format!("d{room}");
            geometry = geometry
                .area(&format!("r{room}"), 40.0, 40.0)
                .walk(&door, "x1", Walk::Between(10.0, 10.0))
                .trace(&door, "cd", 1.0)
                .detour(&door, "x1", "cd", Walk::Unreachable);
        }
        three_rooms_through_one_door()
            .value("cd", "Access", "ClearWidth", metres(width))
            .evaluate_with(
                &EscapeRoute,
                &rule(
                    CAPABILITY,
                    kind("space"),
                    with(
                        doors_and_exits(),
                        vec![
                            uses(&[("area_per_occupant", number(1.0))]),
                            door_widths(&[
                                (60, 0.9, "door_width", 0.9),
                                (200, 1.2, "door_width", 1.2),
                            ]),
                            ("route_door_selector", selector(kind("corridor door"))),
                        ],
                    ),
                ),
                |services| geometry.register(services),
            )
    };
    // 120 occupants need 1.2 m, though each room's 40 alone need 0.9 m.
    let evaluation = run(0.9);
    assert_eq!(
        findings(&evaluation),
        [(
            "cd".into(),
            format!(
                "door {} is 0.9 m wide (stated clear width); 120 occupant(s) relying on it \
                 (from {}, {}, {}) require at least 1.2 m",
                id("cd"),
                id("r1"),
                id("r2"),
                id("r3")
            )
        )]
    );
    assert_eq!(
        evaluation.findings()[0].related,
        vec![id("r1"), id("r2"), id("r3")]
    );
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:?}");
    let evaluation = run(1.2);
    assert!(
        evaluation.findings().is_empty() && unevaluated(&evaluation).is_empty(),
        "{evaluation:?}"
    );
}

#[test]
fn a_rooms_own_doors_together_are_as_wide_as_its_occupants_need() {
    // 150 occupants need 1.8 m of door width together; two 0.8 m doors
    // give 1.6 m.
    let evaluation = Model::default()
        .object("r1", "space")
        .object("d1", "door")
        .object("d2", "door")
        .object("x1", "exit")
        .edge("bounds", "d1", "r1")
        .edge("bounds", "d2", "r1")
        .edge("serves", "x1", "r1")
        .value("x1", "Access", "ClearWidth", metres(2.0))
        .value("d1", "Access", "ClearWidth", metres(0.8))
        .value("d2", "Access", "ClearWidth", metres(0.8))
        .evaluate_with(
            &EscapeRoute,
            &rule(
                CAPABILITY,
                kind("space"),
                with(
                    doors_and_exits(),
                    vec![
                        uses(&[("area_per_occupant", number(1.0))]),
                        door_widths(&[(200, 0.9, "total_door_width", 1.8)]),
                    ],
                ),
            ),
            |services| {
                Geometry::default()
                    .area("r1", 150.0, 150.0)
                    .register(services);
            },
        );
    assert_eq!(
        findings(&evaluation),
        [(
            "r1".into(),
            "its 2 door(s) are at most 1.6 m wide together; 150 occupant(s) require at least \
             1.8 m of door width together (use 0)"
                .into()
        )]
    );
}
