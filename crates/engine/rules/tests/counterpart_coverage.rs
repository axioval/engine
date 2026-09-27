//! Coverage and conformity of walls by their structural counterparts.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    Bounds3, CapabilityEvaluation, CompiledRule, ElevationCover, ElevationInterval,
    ElevationRequest, GeometryFidelity, ObjectBounds, PlanArea, PlanAreaError, PlanAreaService,
    PlanAreaServiceHandle, PlanLength, PlanRectangle, PlanSpan, PlanSpanError, PlanSpanService,
    PlanSpanServiceHandle, ProximityError, ProximityEvidence, ProximityRequest, ProximityService,
    ProximityServiceHandle, RectangleOrientation, VerticalExtent, VerticalExtentError,
    VerticalExtentService, VerticalExtentServiceHandle,
};
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, Severity};
use axioval_rules::CounterpartCoverage;
use common::{Model, id, kind, number, rule, selector, source, string, unevaluated};

const ID: &str = "axioval:capability.counterpart-coverage";

/// A box: plan rectangle `(x0, y0, x1, y1)`, bottom and top elevation, and
/// the chord deviation of a tessellated mesh (zero when exact).
#[derive(Clone, Copy)]
struct Body {
    plan: [f64; 4],
    bottom: f64,
    top: f64,
    deviation: f64,
}

/// Axis-aligned boxes. A stub: it grows a rectangle into a rectangle, which
/// is all these axis-aligned cases need.
#[derive(Default)]
struct Boxes(BTreeMap<ObjectId, Body>);

impl Boxes {
    fn with(mut self, local: &str, plan: [f64; 4], bottom: f64, top: f64) -> Self {
        self.0.insert(
            id(local),
            Body {
                plan,
                bottom,
                top,
                deviation: 0.0,
            },
        );
        self
    }

    fn tessellated(mut self, local: &str, deviation: f64) -> Self {
        self.0.get_mut(&id(local)).unwrap().deviation = deviation;
        self
    }

    fn get(&self, object: &ObjectId) -> Result<Body, PlanAreaError> {
        self.0
            .get(object)
            .copied()
            .ok_or_else(|| PlanAreaError::UnknownObject(object.clone()))
    }
}

fn area(value: f64, slack: f64, locator: String) -> Result<PlanArea, PlanAreaError> {
    let mut evidence = Evidence::exact(source(), locator);
    evidence.exact = slack == 0.0;
    PlanArea::try_new((value - slack).max(0.0), value + slack, evidence)
}

/// The area of `plan` outside every rectangle of `cover`.
fn outside(plan: [f64; 4], cover: &[[f64; 4]]) -> f64 {
    let clip = |range: (f64, f64)| (range.0.max(plan[0]), range.1.min(plan[2]));
    let mut xs = vec![plan[0], plan[2]];
    let mut ys = vec![plan[1], plan[3]];
    for rect in cover {
        let (x0, x1) = clip((rect[0], rect[2]));
        xs.extend([x0, x1]);
        ys.extend([rect[1].max(plan[1]), rect[3].min(plan[3])]);
    }
    xs.retain(|x| (plan[0]..=plan[2]).contains(x));
    ys.retain(|y| (plan[1]..=plan[3]).contains(y));
    xs.sort_by(f64::total_cmp);
    ys.sort_by(f64::total_cmp);
    let mut sum = 0.0;
    for x in xs.windows(2) {
        for y in ys.windows(2) {
            let (cx, cy) = (f64::midpoint(x[0], x[1]), f64::midpoint(y[0], y[1]));
            let covered = cover
                .iter()
                .any(|r| r[0] <= cx && cx <= r[2] && r[1] <= cy && cy <= r[3]);
            if !covered {
                sum += (x[1] - x[0]) * (y[1] - y[0]);
            }
        }
    }
    sum
}

impl PlanAreaService for Boxes {
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        let body = self.get(object)?;
        let [x0, y0, x1, y1] = body.plan;
        area(
            (x1 - x0) * (y1 - y0),
            body.deviation,
            format!("footprint:{object}"),
        )
    }

    fn measure_plan_overlap(&self, _: &ObjectId, _: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        unreachable!("coverage measures uncovered areas, not overlaps")
    }

    fn measure_uncovered_area(
        &self,
        object: &ObjectId,
        cover: &[ObjectId],
        growth_metres: f64,
    ) -> Result<PlanArea, PlanAreaError> {
        let body = self.get(object)?;
        let mut slack = body.deviation;
        let mut rects = Vec::new();
        for member in cover {
            let member = self.get(member)?;
            slack += member.deviation;
            let [x0, y0, x1, y1] = member.plan;
            rects.push([
                x0 - growth_metres,
                y0 - growth_metres,
                x1 + growth_metres,
                y1 + growth_metres,
            ]);
        }
        area(
            outside(body.plan, &rects),
            slack,
            format!("uncovered:{object}"),
        )
    }

    /// Boxes seen along a coordinate axis: a cover box counts when its
    /// depth meets the subject's widened by the along growth, and a frame
    /// by the box around its members, which is their hull for these cases.
    fn measure_elevation_cover(
        &self,
        request: &ElevationRequest,
    ) -> Result<ElevationCover, PlanAreaError> {
        let along_x = request.axis()[0].abs() > 0.5;
        let view = |body: &Body| {
            let [x0, y0, x1, y1] = body.plan;
            let (s, depth) = if along_x {
                ((x0, x1), (y0, y1))
            } else {
                ((y0, y1), (x0, x1))
            };
            ([s.0, body.bottom, s.1, body.top], depth)
        };
        let subject = self.get(request.object())?;
        let (face, depth) = view(&subject);
        let (a, b) = (
            request.along_growth_metres(),
            request.vertical_growth_metres(),
        );
        let mut slack = subject.deviation;
        let mut near = |objects: &[ObjectId]| -> Result<Vec<[f64; 4]>, PlanAreaError> {
            let mut rects = Vec::new();
            for member in objects {
                let member = self.get(member)?;
                slack += member.deviation;
                let (rect, across) = view(&member);
                if across.1 >= depth.0 - a && across.0 <= depth.1 + a {
                    rects.push(rect);
                }
            }
            Ok(rects)
        };
        let mut rects = near(request.cover())?;
        let framed = near(request.frame())?;
        if !framed.is_empty() {
            rects.push(framed.iter().fold(
                [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ],
                |[s0, z0, s1, z1], r| [s0.min(r[0]), z0.min(r[1]), s1.max(r[2]), z1.max(r[3])],
            ));
        }
        let grown: Vec<[f64; 4]> = rects
            .iter()
            .map(|r| [r[0] - a, r[1] - b, r[2] + a, r[3] + b])
            .collect();
        let whole = (face[2] - face[0]) * (face[3] - face[1]);
        let open = outside(face, &grown);
        let mut evidence = Evidence::exact(source(), format!("elevation:{}", request.object()));
        evidence.exact = slack == 0.0;
        ElevationCover::try_new(
            request.object().clone(),
            (
                (whole - subject.deviation).max(0.0),
                whole + subject.deviation,
            ),
            (
                (open - slack).max(0.0),
                (open + slack).min(whole + subject.deviation),
            ),
            evidence,
        )
    }
}

impl ProximityService for Boxes {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        let body = self.get(object).map_err(|_| ProximityError::Unavailable)?;
        let [x0, y0, x1, y1] = body.plan;
        let fidelity = if body.deviation > 0.0 {
            GeometryFidelity::tessellated(body.deviation)?
        } else {
            GeometryFidelity::Exact
        };
        ObjectBounds::try_new(
            object.clone(),
            Bounds3::try_new([x0, y0, body.bottom], [x1, y1, body.top])?,
            fidelity,
        )
    }

    fn measure_proximity(&self, _: &ProximityRequest) -> Result<ProximityEvidence, ProximityError> {
        unreachable!("coverage never measures pairwise proximity")
    }
}

/// Each box is its own least-area rectangle; a tessellated one has no
/// proven orientation.
impl PlanSpanService for Boxes {
    fn measure_diameter(&self, _: &ObjectId) -> Result<PlanLength, PlanSpanError> {
        unreachable!("coverage measures no diameters")
    }

    fn measure_span(
        &self,
        _: &ObjectId,
        _: &ObjectId,
        _: PlanSpan,
    ) -> Result<PlanLength, PlanSpanError> {
        unreachable!("coverage measures no spans")
    }

    fn measure_rectangle(&self, object: &ObjectId) -> Result<PlanRectangle, PlanSpanError> {
        let body = self
            .get(object)
            .map_err(|_| PlanSpanError::UnknownObject(object.clone()))?;
        let [x0, y0, x1, y1] = body.plan;
        let d = body.deviation;
        let mut evidence = Evidence::exact(source(), format!("rectangle:{object}"));
        evidence.exact = d == 0.0;
        let half = |length: f64| ((length / 2.0 - d).max(0.0), length / 2.0 + d);
        PlanRectangle::try_new(
            object.clone(),
            [f64::midpoint(x0, x1), f64::midpoint(y0, y1)],
            d,
            [[1.0, 0.0], [0.0, 1.0]],
            0.0,
            [half(x1 - x0), half(y1 - y0)],
            if d == 0.0 {
                RectangleOrientation::Unique
            } else {
                RectangleOrientation::Unproven
            },
            evidence,
        )
    }
}

impl VerticalExtentService for Boxes {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        let body = self
            .get(object)
            .map_err(|_| VerticalExtentError::UnknownObject(object.clone()))?;
        let d = body.deviation;
        let mut evidence = Evidence::exact(source(), format!("extent:{object}"));
        evidence.exact = d == 0.0;
        VerticalExtent::try_new(
            object.clone(),
            ElevationInterval::try_new(body.bottom - d, body.bottom + d)?,
            ElevationInterval::try_new(body.top - d, body.top + d)?,
            evidence,
        )
    }
}

fn metres(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "m".into(),
    }
}

/// Walls against columns and structural walls (`structure`), graded info
/// above 1 %, warning above 25 % and error above 75 % uncovered.
fn coverage_rule(tolerances: Vec<(&'static str, ParameterValue)>) -> CompiledRule {
    let mut parameters = vec![
        ("counterparts", selector(kind("structure"))),
        ("info_above", number(0.01)),
        ("warning_above", number(0.25)),
        ("error_above", number(0.75)),
    ];
    parameters.extend(tolerances);
    rule(ID, kind("wall"), parameters)
}

fn model(boxes: &Boxes) -> Model {
    boxes.0.keys().fold(Model::default(), |model, object| {
        let kind = match object.local_id.as_bytes()[0] {
            b'w' => "wall",
            b'b' => "beam",
            _ => "structure",
        };
        model.object(&object.local_id, kind)
    })
}

fn run_with(model: Model, boxes: Boxes, rule: &CompiledRule) -> CapabilityEvaluation {
    let shared = Arc::new(boxes);
    model.evaluate_with(&CounterpartCoverage, rule, |services| {
        services
            .register(PlanAreaServiceHandle::new(shared.clone()))
            .unwrap();
        services
            .register(ProximityServiceHandle::new(shared.clone()))
            .unwrap();
        services
            .register(VerticalExtentServiceHandle::new(shared.clone()))
            .unwrap();
        services
            .register(PlanSpanServiceHandle::new(shared))
            .unwrap();
    })
}

fn run(boxes: Boxes, rule: &CompiledRule) -> CapabilityEvaluation {
    run_with(model(&boxes), boxes, rule)
}

/// `(object, severity, message)` of every finding, sorted.
fn graded(evaluation: &CapabilityEvaluation) -> Vec<(String, Severity, String)> {
    let mut found: Vec<_> = evaluation
        .findings()
        .iter()
        .map(|finding| {
            (
                common::subject(finding),
                finding.severity.clone(),
                finding.message.clone(),
            )
        })
        .collect();
    found.sort_by(|a, b| (&a.0, &a.2).cmp(&(&b.0, &b.2)));
    found
}

/// Three 4 m walls, 0.2 m thick and 3 m high: `w1` stands on a structural
/// wall of its own size, `w2` on nothing, and `w3` on one along half of it.
fn three_walls() -> Boxes {
    Boxes::default()
        .with("w1", [0.0, 0.0, 4.0, 0.2], 0.0, 3.0)
        .with("s1", [0.0, 0.0, 4.0, 0.2], 0.0, 3.0)
        .with("w2", [0.0, 5.0, 4.0, 5.2], 0.0, 3.0)
        .with("w3", [0.0, 10.0, 4.0, 10.2], 0.0, 3.0)
        .with("s3", [0.0, 10.0, 2.0, 10.2], 0.0, 3.0)
}

#[test]
fn a_wall_with_no_counterpart_and_a_partly_covered_one_are_graded() {
    let evaluation = run(
        three_walls(),
        &coverage_rule(vec![("tolerance", metres(0.02))]),
    );
    let found = graded(&evaluation);
    assert_eq!(found.len(), 3, "{found:#?}");
    // Nothing stands under w2: all of it is uncovered, in plan and height.
    assert_eq!(found[0].0, "w2");
    assert_eq!(found[0].1, Severity::Error);
    assert_eq!(
        found[0].2,
        "height: 1 of the height (3 of 3 m) lies outside every counterpart overlapping it in \
         plan, grown by 0.02 m; no counterpart overlaps it"
    );
    assert_eq!(
        found[1],
        (
            "w2".to_owned(),
            Severity::Error,
            "plan: 1 of the footprint (0.8 of 0.8 m²) lies outside every counterpart grown by \
             0.02 m; no counterpart overlaps it"
                .to_owned()
        )
    );
    // Half of w3 is uncovered in plan, less the 2 cm the grown column
    // reaches past its end; its height is covered where they meet.
    assert_eq!(found[2].0, "w3");
    assert_eq!(found[2].1, Severity::Warning);
    assert!(
        found[2]
            .2
            .starts_with("plan: 0.495 of the footprint (0.396 of 0.8 m²)"),
        "{}",
        found[2].2
    );
    assert_eq!(
        evaluation.findings()[2].related,
        vec![id("s3")],
        "the counterpart is named"
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn conformity_takes_separate_tolerances_and_a_negative_one_switches_a_check_off() {
    // A structural wall shifted 3 cm across and 0.5 m lower than w1.
    let shifted = || {
        Boxes::default()
            .with("w1", [0.0, 0.0, 4.0, 0.2], 0.0, 3.0)
            .with("s1", [0.0, 0.03, 4.0, 0.23], 0.0, 2.5)
    };
    // Within 5 cm across it conforms in plan; its height is not checked.
    let plan_only = coverage_rule(vec![
        ("horizontal_tolerance", metres(0.05)),
        ("vertical_tolerance", metres(-1.0)),
    ]);
    let evaluation = run(shifted(), &plan_only);
    assert!(graded(&evaluation).is_empty(), "{:#?}", graded(&evaluation));
    assert!(evaluation.not_evaluated_outcomes().is_empty());

    // With 1 cm across, a 2 cm strip stays uncovered: 10 %, info.
    let tight = coverage_rule(vec![
        ("horizontal_tolerance", metres(0.01)),
        ("vertical_tolerance", metres(-1.0)),
    ]);
    let found = graded(&run(shifted(), &tight));
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].1, Severity::Info);
    assert!(
        found[0].2.starts_with("plan: 0.1 of the footprint"),
        "{found:#?}"
    );

    // Height only: the top 0.5 m of 3 m is uncovered, or 0.4 m within 10 cm.
    let height_only = |vertical: f64| {
        coverage_rule(vec![
            ("horizontal_tolerance", metres(-1.0)),
            ("vertical_tolerance", metres(vertical)),
        ])
    };
    let found = graded(&run(shifted(), &height_only(0.0)));
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0]
            .2
            .starts_with("height: 0.1667 of the height (0.5 of 3 m)"),
        "{found:#?}"
    );
    let found = graded(&run(shifted(), &height_only(0.1)));
    assert!(
        found[0]
            .2
            .starts_with("height: 0.1333 of the height (0.4 of 3 m)"),
        "{found:#?}"
    );
    assert_eq!(found[0].1, Severity::Info);
}

#[test]
fn one_tolerance_covers_both_checks() {
    let shifted = Boxes::default()
        .with("w1", [0.0, 0.0, 4.0, 0.2], 0.0, 3.0)
        .with("s1", [0.0, 0.03, 4.0, 0.23], 0.0, 2.95);
    // 5 cm covers both the 3 cm shift and the 5 cm short top.
    let evaluation = run(shifted, &coverage_rule(vec![("tolerance", metres(0.05))]));
    assert!(graded(&evaluation).is_empty(), "{:#?}", graded(&evaluation));
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

/// Structure is `structure`, or anything whose `Pset.LoadBearing` exists.
fn structure_or_load_bearing() -> Selector {
    Selector::AnyOf {
        operands: vec![
            kind("structure"),
            Selector::Property {
                property_set: Some("Pset".into()),
                property: "LoadBearing".into(),
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
fn an_undecided_counterpart_leaves_only_a_pass_standing() {
    // `b2` might be structure under w2; w1 is covered either way.
    let boxes = three_walls().with("b2", [0.0, 5.0, 4.0, 5.2], 0.0, 3.0);
    let model = model(&boxes).unreadable("b2");
    let mut declared = coverage_rule(vec![("tolerance", metres(0.02))]);
    declared
        .parameters
        .insert("counterparts".into(), selector(structure_or_load_bearing()));
    let evaluation = run_with(model, boxes, &declared);
    let found = graded(&evaluation);
    // w3's half cover is a finding whatever b2 is: it stands far away.
    assert_eq!(
        found.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(),
        ["w3"],
        "{found:#?}"
    );
    let outcomes = unevaluated(&evaluation);
    assert_eq!(
        outcomes,
        [
            ("w2".to_owned(), NotEvaluatedReason::IncompleteEvidence),
            ("w2".to_owned(), NotEvaluatedReason::IncompleteEvidence),
        ],
        "{outcomes:#?}"
    );
    assert!(
        evaluation.not_evaluated_outcomes()[0]
            .message()
            .contains("straddles the threshold 0.01"),
        "{:?}",
        evaluation.not_evaluated_outcomes()
    );
}

#[test]
fn a_counterpart_without_a_readable_extent_leaves_only_a_pass_standing() {
    let boxes = three_walls();
    // `s9` is selected but has no geometry: it may stand anywhere.
    let model = model(&boxes).object("s9", "structure");
    let evaluation = run_with(
        model,
        boxes,
        &coverage_rule(vec![("tolerance", metres(0.02))]),
    );
    assert!(graded(&evaluation).is_empty(), "{:#?}", graded(&evaluation));
    let mut objects: Vec<String> = unevaluated(&evaluation)
        .into_iter()
        .map(|(object, _)| object)
        .collect();
    objects.dedup();
    // w1 passes; w2 and w3 might be covered by s9.
    assert_eq!(objects, ["w2", "w3"]);
    assert!(
        evaluation.not_evaluated_outcomes()[0]
            .message()
            .contains("1 counterpart(s) have no readable extent")
    );
}

#[test]
fn a_share_straddling_a_higher_threshold_is_graded_by_its_upper_bound() {
    // A tessellated counterpart under 70 % of the wall: 30 % uncovered,
    // give or take its deviation's slack.
    let boxes = Boxes::default()
        .with("w1", [0.0, 0.0, 4.0, 0.2], 0.0, 3.0)
        .with("s1", [0.0, 0.0, 2.8, 0.2], 0.0, 3.0)
        .tessellated("s1", 0.05);
    let rule = coverage_rule(vec![
        ("horizontal_tolerance", metres(0.0)),
        ("vertical_tolerance", metres(-1.0)),
    ]);
    let found = graded(&run(boxes, &rule));
    assert_eq!(found.len(), 1, "{found:#?}");
    // Between 0.24 and 0.36 m² of 0.8: 0.3 ± 0.0625, across 25 %.
    assert_eq!(found[0].1, Severity::Warning);
    assert!(
        found[0]
            .2
            .ends_with("graded warning by its upper bound, at least info"),
        "{found:#?}"
    );
}

#[test]
fn invalid_declarations_refuse_the_rule() {
    for (tolerances, bands) in [
        (vec![], vec![("info_above", number(0.0))]),
        (
            vec![
                ("tolerance", metres(0.02)),
                ("horizontal_tolerance", metres(0.02)),
            ],
            vec![("info_above", number(0.0))],
        ),
        (
            vec![("horizontal_tolerance", metres(0.02))],
            vec![("info_above", number(0.0))],
        ),
        (
            vec![("tolerance", metres(-1.0))],
            vec![("info_above", number(0.0))],
        ),
        (
            vec![(
                "tolerance",
                ParameterValue::Quantity {
                    value: 1.0,
                    unit: "m2".into(),
                },
            )],
            vec![("info_above", number(0.0))],
        ),
        (vec![("tolerance", metres(0.0))], vec![]),
        (
            vec![("tolerance", metres(0.0))],
            vec![("info_above", number(0.5)), ("error_above", number(0.5))],
        ),
        (
            vec![("tolerance", metres(0.0))],
            vec![("warning_above", number(1.0))],
        ),
    ] {
        let mut parameters = vec![("counterparts", selector(kind("structure")))];
        parameters.extend(tolerances);
        parameters.extend(bands);
        let evaluation = run(three_walls(), &rule(ID, kind("wall"), parameters));
        assert!(evaluation.findings().is_empty());
        let outcomes = unevaluated(&evaluation);
        assert_eq!(
            outcomes,
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)],
            "{:?}",
            evaluation.not_evaluated_outcomes()
        );
    }
}

#[test]
fn a_missing_service_leaves_every_wall_not_evaluated() {
    let evaluation = model(&three_walls()).evaluate(
        &CounterpartCoverage,
        &coverage_rule(vec![("tolerance", metres(0.0))]),
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        ["w1", "w2", "w3"].map(|wall| (wall.to_owned(), NotEvaluatedReason::MissingService))
    );
}

fn degrees(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "deg".into(),
    }
}

/// `w1` along x with `p1`, a structural wall across it, standing on its
/// middle.
fn crossed() -> Boxes {
    Boxes::default()
        .with("w1", [0.0, 0.0, 4.0, 0.2], 0.0, 3.0)
        .with("p1", [1.9, -1.0, 2.1, 1.2], 0.0, 3.0)
}

#[test]
fn a_perpendicular_counterpart_counts_only_without_an_axis_tolerance() {
    // Without an axis tolerance, p1 covers w1's height where they cross.
    let found = graded(&run(
        crossed(),
        &coverage_rule(vec![("tolerance", metres(0.0))]),
    ));
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0].2.starts_with("plan: 0.95 of the footprint"),
        "{found:#?}"
    );

    // Within 5 degrees of parallel only, p1 is no counterpart at all.
    let evaluation = run(
        crossed(),
        &coverage_rule(vec![
            ("tolerance", metres(0.0)),
            ("axis_tolerance", degrees(5.0)),
        ]),
    );
    let found = graded(&evaluation);
    assert_eq!(found.len(), 2, "{found:#?}");
    assert!(
        found
            .iter()
            .all(|(_, _, message)| message.ends_with("no counterpart overlaps it")),
        "{found:#?}"
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn a_parallel_counterpart_counts_with_an_axis_tolerance() {
    let boxes = three_walls();
    let found = graded(&run(
        boxes,
        &coverage_rule(vec![
            ("tolerance", metres(0.02)),
            ("axis_tolerance", degrees(5.0)),
        ]),
    ));
    // As without the tolerance: every structural wall is parallel.
    assert_eq!(
        found.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(),
        ["w2", "w2", "w3"],
        "{found:#?}"
    );
}

#[test]
fn a_counterpart_without_a_long_axis_is_an_undecided_cover() {
    // A square column under all of w1: it may or may not share its axis.
    let boxes = Boxes::default()
        .with("w1", [0.0, 0.0, 4.0, 0.2], 0.0, 3.0)
        .with("q1", [0.0, -1.9, 4.0, 2.1], 0.0, 3.0);
    let evaluation = run(
        boxes,
        &coverage_rule(vec![
            ("tolerance", metres(0.0)),
            ("axis_tolerance", degrees(5.0)),
        ]),
    );
    assert!(graded(&evaluation).is_empty(), "{:#?}", graded(&evaluation));
    let outcomes = evaluation.not_evaluated_outcomes();
    assert_eq!(outcomes.len(), 2, "{outcomes:?}");
    assert!(
        outcomes
            .iter()
            .all(|outcome| outcome.message().contains("too close to equal")),
        "{outcomes:?}"
    );
}

#[test]
fn an_element_without_proven_axes_leaves_every_cover_undecided() {
    // w1 is tessellated: which way it runs is not proven.
    let boxes = three_walls().tessellated("w1", 0.001);
    let evaluation = run(
        boxes,
        &coverage_rule(vec![
            ("tolerance", metres(0.02)),
            ("axis_tolerance", degrees(5.0)),
        ]),
    );
    let outcomes: Vec<String> = unevaluated(&evaluation)
        .into_iter()
        .map(|(object, _)| object)
        .collect();
    assert_eq!(
        outcomes,
        ["w1", "w1"],
        "{:?}",
        evaluation.not_evaluated_outcomes()
    );
    assert!(
        evaluation.not_evaluated_outcomes()[0]
            .message()
            .contains("is tessellated")
    );
}

/// `w1` stands on a full-height structural wall along its left half and a
/// half-height one along its right half.
fn two_heights() -> Boxes {
    Boxes::default()
        .with("w1", [0.0, 0.0, 4.0, 0.2], 0.0, 3.0)
        .with("s1", [0.0, 0.0, 2.0, 0.2], 0.0, 3.0)
        .with("s2", [2.0, 0.0, 4.0, 0.2], 0.0, 1.5)
}

fn elevation_rule(extra: Vec<(&'static str, ParameterValue)>) -> CompiledRule {
    let mut tolerances = vec![
        ("tolerance", metres(0.02)),
        ("measure", string("elevation")),
    ];
    tolerances.extend(extra);
    coverage_rule(tolerances)
}

#[test]
fn two_heights_pass_plan_and_height_but_fail_in_elevation() {
    let separate = run(
        two_heights(),
        &coverage_rule(vec![("tolerance", metres(0.02))]),
    );
    assert!(graded(&separate).is_empty(), "{:#?}", graded(&separate));
    assert!(separate.not_evaluated_outcomes().is_empty());

    let evaluation = run(two_heights(), &elevation_rule(vec![]));
    let found = graded(&evaluation);
    assert_eq!(found.len(), 1, "{found:#?}");
    // The upper right quarter, less 2 cm along and in height:
    // 1.98 × 1.48 of 12 m².
    assert_eq!(found[0].1, Severity::Info);
    assert_eq!(
        found[0].2,
        "elevation: 0.2442 of the elevation (2.9304 of 12 m²) lies outside every \
         counterpart, grown by 0.02 m along its axis and 0.02 m in height"
    );
    assert_eq!(evaluation.findings()[0].related, vec![id("s1"), id("s2")]);
    let deviation = evaluation.deviation(0).expect("graded");
    // 0.2442 over the lowest threshold of 0.01.
    assert!((deviation.lower() - 23.42).abs() < 1e-6, "{deviation:?}");
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

/// `w1` fills the bay of two columns and a beam (`bc1`, `bc2`, `bb`).
fn bay() -> Boxes {
    Boxes::default()
        .with("w1", [0.0, 0.0, 4.0, 0.2], 0.0, 3.0)
        .with("bc1", [-0.3, 0.0, 0.0, 0.2], 0.0, 3.0)
        .with("bc2", [4.0, 0.0, 4.3, 0.2], 0.0, 3.0)
        .with("bb", [-0.3, 0.0, 4.3, 0.2], 3.0, 3.4)
}

#[test]
fn a_wall_filling_a_frame_passes_with_infill_on() {
    let without = graded(&run(bay(), &elevation_rule(vec![])));
    assert_eq!(without.len(), 1, "{without:#?}");
    assert_eq!(without[0].1, Severity::Error);

    let evaluation = run(
        bay(),
        &elevation_rule(vec![("infill_counterparts", selector(kind("beam")))]),
    );
    assert!(graded(&evaluation).is_empty(), "{:#?}", graded(&evaluation));
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn the_infill_covers_only_above_its_share() {
    // A structural wall under 60 % of w1 leaves less than half uncovered,
    // so the frame's infill does not count.
    let boxes = bay().with("s1", [0.0, 0.0, 2.4, 0.2], 0.0, 3.0);
    let found = graded(&run(
        boxes,
        &elevation_rule(vec![("infill_counterparts", selector(kind("beam")))]),
    ));
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0]
            .2
            .starts_with("elevation: 0.395 of the elevation (4.74 of 12 m²)"),
        "{found:#?}"
    );
    // Above a share of a third it does.
    let boxes = bay().with("s1", [0.0, 0.0, 2.4, 0.2], 0.0, 3.0);
    let evaluation = run(
        boxes,
        &elevation_rule(vec![
            ("infill_counterparts", selector(kind("beam"))),
            ("infill_above", number(0.3)),
        ]),
    );
    assert!(graded(&evaluation).is_empty(), "{:#?}", graded(&evaluation));
}

#[test]
fn an_element_without_a_long_axis_has_no_elevation() {
    let boxes = Boxes::default()
        .with("w1", [0.0, 0.0, 1.0, 1.0], 0.0, 3.0)
        .with("s1", [0.0, 0.0, 1.0, 1.0], 0.0, 3.0);
    let evaluation = run(boxes, &elevation_rule(vec![]));
    assert!(evaluation.findings().is_empty());
    let outcomes = evaluation.not_evaluated_outcomes();
    assert_eq!(outcomes.len(), 1, "{outcomes:?}");
    assert!(
        outcomes[0].message().contains("no long axis"),
        "{outcomes:?}"
    );
}

#[test]
fn invalid_elevation_declarations_refuse_the_rule() {
    for parameters in [
        vec![("tolerance", metres(0.02)), ("measure", string("section"))],
        vec![
            ("measure", string("elevation")),
            ("horizontal_tolerance", metres(0.02)),
            ("vertical_tolerance", metres(-1.0)),
        ],
        vec![
            ("tolerance", metres(0.02)),
            ("infill_counterparts", selector(kind("beam"))),
        ],
        vec![
            ("tolerance", metres(0.02)),
            ("measure", string("elevation")),
            ("infill_above", number(0.5)),
        ],
        vec![
            ("tolerance", metres(0.02)),
            ("measure", string("elevation")),
            ("infill_counterparts", selector(kind("beam"))),
            ("infill_above", number(1.0)),
        ],
    ] {
        let evaluation = run(bay(), &coverage_rule(parameters));
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)],
            "{:?}",
            evaluation.not_evaluated_outcomes()
        );
    }
}
