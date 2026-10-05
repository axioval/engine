//! `body-extent`: a body's depth along its own placement axis, against the
//! thickness its material layers state or a range.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, DirectionalExtent, ElevationInterval, MetricDirection, MetricFrame,
    MetricPoint, ObjectFrame, ObjectFrameError, ObjectFrameService, ObjectFrameServiceHandle,
    ObjectFront, ServiceRegistry, SourceSnapshot, VerticalExtent, VerticalExtentError,
    VerticalExtentService, VerticalExtentServiceHandle,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, PropertyValue, QuantityDimension};
use axioval_rules::BodyExtent;
use common::{Model, findings, id, kind, property, rule, source, string, unevaluated};

const ID: &str = "axioval:capability.body-extent";
const MATERIAL: &str = "axioval:material";

fn direction(vector: [f64; 3]) -> MetricDirection {
    MetricDirection::try_new(vector).unwrap()
}

/// Placement frames: each object's right axis, or none when unplaced.
#[derive(Default)]
struct Frames {
    snapshots: Vec<SourceSnapshot>,
    right: BTreeMap<ObjectId, [f64; 3]>,
}

impl Frames {
    fn new() -> Self {
        Self {
            snapshots: vec![SourceSnapshot::try_new(source(), "r1", "sha256:1").unwrap()],
            right: BTreeMap::new(),
        }
    }

    fn with(mut self, local: &str, right: [f64; 3]) -> Self {
        self.right.insert(id(local), right);
        self
    }
}

impl ObjectFrameService for Frames {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn object_frame(&self, object: &ObjectId) -> Result<ObjectFrame, ObjectFrameError> {
        let right = *self
            .right
            .get(object)
            .ok_or_else(|| ObjectFrameError::NotPlaced(object.clone()))?;
        let up = [0.0, 0.0, 1.0];
        // forward = up × right, so the frame is right-handed.
        let forward = [-right[1], right[0], 0.0];
        let frame = MetricFrame::try_new(
            MetricPoint::try_new(object.clone(), [0.0; 3]).unwrap(),
            direction(right),
            direction(forward),
            direction(up),
        )
        .unwrap();
        ObjectFrame::try_new(
            object.clone(),
            frame,
            ObjectFront::NotStated,
            Evidence::exact(source(), format!("placement:{}", object.local_id)),
        )
    }
}

/// World-aligned boxes: dimensions in x, y and z, and an uncertainty added
/// to both positions. A box's extent along a unit direction `d` is
/// `Σ |d_i| · size_i`.
#[derive(Default)]
struct Boxes(BTreeMap<ObjectId, ([f64; 3], f64)>);

impl Boxes {
    fn with(mut self, local: &str, size: [f64; 3], slack: f64) -> Self {
        self.0.insert(id(local), (size, slack));
        self
    }
}

impl VerticalExtentService for Boxes {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        Err(VerticalExtentError::UnknownObject(object.clone()))
    }

    fn measure_directional_extent(
        &self,
        object: &ObjectId,
        along: MetricDirection,
    ) -> Result<DirectionalExtent, VerticalExtentError> {
        let (size, slack) = *self
            .0
            .get(object)
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?;
        let d = along.components();
        let length: f64 = (0..3).map(|i| d[i].abs() * size[i]).sum();
        let mut evidence = Evidence::exact(source(), format!("extent:{}", object.local_id));
        evidence.exact = slack == 0.0;
        DirectionalExtent::try_new(
            object.clone(),
            along,
            ElevationInterval::try_new(-slack, slack)?,
            ElevationInterval::try_new(length - slack, length + slack)?,
            evidence,
        )
    }
}

fn metres(value: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value,
        dimension: QuantityDimension::Length,
    }
}

fn length(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "m".into(),
    }
}

fn millimetres(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "mm".into(),
    }
}

fn thickness() -> ParameterValue {
    property(Some(MATERIAL), "TotalThickness")
}

fn run(
    model: Model,
    frames: Frames,
    boxes: Boxes,
    parameters: Vec<(&str, ParameterValue)>,
) -> CapabilityEvaluation {
    model.evaluate_with(
        &BodyExtent,
        &rule(ID, kind("wall"), parameters),
        |services: &mut ServiceRegistry| {
            services
                .register(ObjectFrameServiceHandle::new(Arc::new(frames)))
                .unwrap();
            services
                .register(VerticalExtentServiceHandle::new(Arc::new(boxes)))
                .unwrap();
        },
    )
}

/// Walls along x (`right` = x, so `forward` = y) and one along y.
fn walls() -> (Model, Frames, Boxes) {
    let model = Model::default()
        .object("fits", "wall")
        .object("thin", "wall")
        .object("turned", "wall")
        .object("bare", "wall")
        .value("fits", MATERIAL, "TotalThickness", metres(0.3))
        .value("thin", MATERIAL, "TotalThickness", metres(0.3))
        .value("turned", MATERIAL, "TotalThickness", metres(0.24));
    let frames = Frames::new()
        .with("fits", [1.0, 0.0, 0.0])
        .with("thin", [1.0, 0.0, 0.0])
        .with("turned", [0.0, 1.0, 0.0])
        .with("bare", [1.0, 0.0, 0.0]);
    let boxes = Boxes::default()
        .with("fits", [5.0, 0.3, 3.0], 0.0)
        .with("thin", [5.0, 0.25, 3.0], 0.0)
        // Runs along y, 0.24 m thick in x.
        .with("turned", [0.24, 5.0, 3.0], 0.0)
        .with("bare", [5.0, 0.3, 3.0], 0.0);
    (model, frames, boxes)
}

#[test]
fn a_body_thicker_or_thinner_than_its_layers_is_found() {
    let (model, frames, boxes) = walls();
    let evaluation = run(
        model,
        frames,
        boxes,
        vec![
            ("axis", string("forward")),
            ("target_property", thickness()),
        ],
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "bare".into(),
                "body extent along `forward` is 0.3 m; `axioval:material.TotalThickness` is \
                 absent"
                    .into()
            ),
            (
                "thin".into(),
                "body extent along `forward` is 0.25 m; `axioval:material.TotalThickness` states \
                 0.3 m"
                    .into()
            ),
        ]
    );
    // The frame, the measurement and the stated thickness are all cited.
    let locators: Vec<&str> = evaluation.findings()[1]
        .evidence
        .iter()
        .map(|evidence| evidence.locator.as_str())
        .collect();
    assert!(locators.contains(&"placement:thin") && locators.contains(&"extent:thin"));
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

/// The axis is the object's own: along world y the turned wall measures
/// 5 m, along its `forward` axis its 0.24 m thickness.
#[test]
fn the_axis_is_the_objects_own_placement_axis() {
    let (model, frames, boxes) = walls();
    let evaluation = run(
        model,
        frames,
        boxes,
        vec![("axis", string("right")), ("target_property", thickness())],
    );
    let flagged: Vec<String> = findings(&evaluation)
        .into_iter()
        .map(|(object, _)| object)
        .collect();
    assert_eq!(flagged, ["bare", "fits", "thin", "turned"]);
    assert!(
        findings(&evaluation)[3]
            .1
            .starts_with("body extent along `right` is 5 m")
    );
}

#[test]
fn a_tolerance_accepts_a_body_close_to_its_layers() {
    let (model, frames, boxes) = walls();
    let evaluation = run(
        model,
        frames,
        boxes,
        vec![
            ("axis", string("forward")),
            ("target_property", thickness()),
            ("tolerance", millimetres(50.0)),
        ],
    );
    assert_eq!(
        findings(&evaluation)
            .into_iter()
            .map(|(object, _)| object)
            .collect::<Vec<_>>(),
        ["bare"]
    );
    let evaluation = run(
        walls().0,
        walls().1,
        walls().2,
        vec![
            ("axis", string("forward")),
            ("target_property", thickness()),
            ("tolerance", millimetres(10.0)),
        ],
    );
    assert!(
        findings(&evaluation)[1]
            .1
            .ends_with("states 0.3 m within 0.01 m"),
        "{:?}",
        findings(&evaluation)
    );
}

/// An exact extent that differs from the target only by binary rounding
/// is not a finding, even without a tolerance.
#[test]
fn rounding_alone_is_not_a_finding() {
    let model = Model::default().object("w", "wall").value(
        "w",
        MATERIAL,
        "TotalThickness",
        metres(0.1 + 0.2),
    );
    let evaluation = run(
        model,
        Frames::new().with("w", [1.0, 0.0, 0.0]),
        Boxes::default().with("w", [5.0, 0.3, 3.0], 0.0),
        vec![
            ("axis", string("forward")),
            ("target_property", thickness()),
        ],
    );
    assert!(evaluation.findings().is_empty());
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn an_extent_straddling_the_stated_thickness_is_not_evaluated() {
    let model = Model::default()
        .object("curved", "wall")
        .object("far", "wall")
        .value("curved", MATERIAL, "TotalThickness", metres(0.3))
        .value("far", MATERIAL, "TotalThickness", metres(0.3));
    let evaluation = run(
        model,
        Frames::new()
            .with("curved", [1.0, 0.0, 0.0])
            .with("far", [1.0, 0.0, 0.0]),
        Boxes::default()
            .with("curved", [5.0, 0.3, 3.0], 0.01)
            .with("far", [5.0, 0.5, 3.0], 0.01),
        vec![
            ("axis", string("forward")),
            ("target_property", thickness()),
        ],
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("curved".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    // Far enough off, an interval still decides; its evidence says it is
    // approximate.
    assert_eq!(
        findings(&evaluation),
        [(
            "far".into(),
            "body extent along `forward` is between 0.48 m and 0.52 m; \
             `axioval:material.TotalThickness` states 0.3 m"
                .into()
        )]
    );
    assert!(
        evaluation.findings()[0]
            .evidence
            .iter()
            .any(|evidence| !evidence.exact)
    );
}

#[test]
fn a_range_bounds_the_extent() {
    let (model, frames, boxes) = walls();
    let evaluation = run(
        model,
        frames,
        boxes,
        vec![
            ("axis", string("forward")),
            ("minimum", length(0.26)),
            ("maximum", millimetres(400.0)),
        ],
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "thin".into(),
                "body extent along `forward` is 0.25 m; required at least 0.26 m".into()
            ),
            (
                "turned".into(),
                "body extent along `forward` is 0.24 m; required at least 0.26 m".into()
            ),
        ]
    );
}

#[test]
fn unplaced_unmeasured_and_non_length_targets_are_not_evaluated() {
    let model = Model::default()
        .object("unplaced", "wall")
        .object("unmeshed", "wall")
        .object("counted", "wall")
        .value("unplaced", MATERIAL, "TotalThickness", metres(0.3))
        .value("unmeshed", MATERIAL, "TotalThickness", metres(0.3))
        .value(
            "counted",
            MATERIAL,
            "TotalThickness",
            PropertyValue::Decimal(0.3),
        );
    let evaluation = run(
        model,
        Frames::new()
            .with("unmeshed", [1.0, 0.0, 0.0])
            .with("counted", [1.0, 0.0, 0.0]),
        Boxes::default().with("counted", [5.0, 0.3, 3.0], 0.0),
        vec![
            ("axis", string("forward")),
            ("target_property", thickness()),
        ],
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("counted".into(), NotEvaluatedReason::InvalidEvidence),
            ("unmeshed".into(), NotEvaluatedReason::BackendUnavailable),
            ("unplaced".into(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
}

#[test]
fn declarations_that_cannot_be_judged_are_refused() {
    for parameters in [
        vec![
            ("axis", string("sideways")),
            ("target_property", thickness()),
        ],
        vec![("axis", string("forward"))],
        vec![
            ("axis", string("forward")),
            ("target_property", thickness()),
            ("maximum", length(1.0)),
        ],
        vec![
            ("axis", string("forward")),
            ("maximum", length(1.0)),
            ("tolerance", length(0.01)),
        ],
        vec![
            ("axis", string("forward")),
            ("minimum", length(2.0)),
            ("maximum", length(1.0)),
        ],
        vec![
            ("axis", string("forward")),
            (
                "maximum",
                ParameterValue::Quantity {
                    value: 1.0,
                    unit: "m2".into(),
                },
            ),
        ],
    ] {
        let (model, frames, boxes) = walls();
        let evaluation = run(model, frames, boxes, parameters);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

#[test]
fn without_its_services_nothing_is_judged() {
    let (model, _, _) = walls();
    let evaluation = model.evaluate(
        &BodyExtent,
        &rule(
            ID,
            kind("wall"),
            vec![
                ("axis", string("forward")),
                ("target_property", thickness()),
            ],
        ),
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::MissingService)]
    );
}

/// The measured value `extent` along an own axis reads what `body-extent`
/// judges: the same extents on the same fixtures.
#[test]
fn the_measured_extent_along_an_own_axis_is_the_body_extent() {
    use axioval_engine::{PropertyResolution, measured_value};
    let (model, frames, boxes) = walls();
    let project = model.project();
    let mut services = ServiceRegistry::new();
    services
        .register(ObjectFrameServiceHandle::new(Arc::new(frames)))
        .unwrap();
    services
        .register(VerticalExtentServiceHandle::new(Arc::new(boxes)))
        .unwrap();
    for (wall, along_forward, along_right) in [
        ("fits", 0.3, 5.0),
        ("thin", 0.25, 5.0),
        ("turned", 0.24, 5.0),
    ] {
        for (name, expected) in [
            ("extent;axis=own_y", along_forward),
            ("extent;axis=own_x", along_right),
            ("length", along_right),
        ] {
            let PropertyResolution::Present(resolved) =
                measured_value(&services, &project, &id(wall), name).unwrap()
            else {
                panic!("{wall} has no {name}");
            };
            assert_eq!(
                resolved.property().value(),
                &metres(expected),
                "{wall} {name}"
            );
        }
    }
    // A world direction reads the box as it lies: the turned wall is 5 m
    // along y, and 0.24 m along x.
    let PropertyResolution::Present(along_y) =
        measured_value(&services, &project, &id("turned"), "extent;direction=0,2,0").unwrap()
    else {
        panic!("no extent");
    };
    assert_eq!(along_y.property().value(), &metres(5.0));
    for refused in ["extent", "extent;axis=own_x;direction=1,0,0"] {
        assert!(
            measured_value(&services, &project, &id("turned"), refused).is_err(),
            "{refused}"
        );
    }
}

/// `body_extent` against the stated thickness within the tolerance, or
/// within the range, rounded to the micrometre as the capability allows
/// binary rounding, reaches `body-extent`'s verdicts on its fixtures.
mod as_expressions {
    use super::*;
    use common::expressions::{
        abs, assert_parity, at_least, at_most, between, m, measured, mm, quantity,
        rule as expression, stated, subtract,
    };
    use serde_json::Value;

    type Fixture = fn() -> (Model, Frames, Boxes);

    fn parity(fixture: Fixture, parameters: Vec<(&str, ParameterValue)>, requirement: &Value) {
        let (model, frames, boxes) = fixture();
        let capability = run(model, frames, boxes, parameters);
        let (model, _, _) = fixture();
        let rewrite = model.evaluate_measured(
            &axioval_rules::ExpressionRequirement,
            &expression(kind("wall"), requirement),
            |services| {
                let (_, frames, boxes) = fixture();
                services
                    .register(ObjectFrameServiceHandle::new(Arc::new(frames)))
                    .unwrap();
                services
                    .register(VerticalExtentServiceHandle::new(Arc::new(boxes)))
                    .unwrap();
            },
        );
        assert_parity(ID, &capability, &rewrite);
    }

    fn extent(axis: &str) -> Value {
        measured(&format!("body_extent;axis={axis}"))
    }

    /// The extent along `axis` equals the stated thickness within
    /// `tolerance`.
    fn as_stated(axis: &str, tolerance: Value) -> Value {
        at_most(
            mm(abs(subtract(
                extent(axis),
                stated(MATERIAL, "TotalThickness"),
            ))),
            tolerance,
        )
    }

    fn straddling() -> (Model, Frames, Boxes) {
        let model = Model::default()
            .object("curved", "wall")
            .object("far", "wall")
            .value("curved", MATERIAL, "TotalThickness", metres(0.3))
            .value("far", MATERIAL, "TotalThickness", metres(0.3));
        (
            model,
            Frames::new()
                .with("curved", [1.0, 0.0, 0.0])
                .with("far", [1.0, 0.0, 0.0]),
            Boxes::default().with("curved", [5.0, 0.3, 3.0], 0.01).with(
                "far",
                [5.0, 0.5, 3.0],
                0.01,
            ),
        )
    }

    fn rounded() -> (Model, Frames, Boxes) {
        let model = Model::default().object("w", "wall").value(
            "w",
            MATERIAL,
            "TotalThickness",
            metres(0.1 + 0.2),
        );
        (
            model,
            Frames::new().with("w", [1.0, 0.0, 0.0]),
            Boxes::default().with("w", [5.0, 0.3, 3.0], 0.0),
        )
    }

    fn unreadable() -> (Model, Frames, Boxes) {
        let model = Model::default()
            .object("unplaced", "wall")
            .object("unmeshed", "wall")
            .object("counted", "wall")
            .value("unplaced", MATERIAL, "TotalThickness", metres(0.3))
            .value("unmeshed", MATERIAL, "TotalThickness", metres(0.3))
            .value(
                "counted",
                MATERIAL,
                "TotalThickness",
                PropertyValue::Decimal(0.3),
            );
        (
            model,
            Frames::new()
                .with("unmeshed", [1.0, 0.0, 0.0])
                .with("counted", [1.0, 0.0, 0.0]),
            Boxes::default().with("counted", [5.0, 0.3, 3.0], 0.0),
        )
    }

    #[test]
    fn the_extent_against_the_stated_thickness_reaches_the_verdicts() {
        let target = || ("target_property", thickness());
        for fixture in [walls as Fixture, straddling, rounded, unreadable] {
            for axis in ["forward", "right", "up"] {
                parity(
                    fixture,
                    vec![("axis", string(axis)), target()],
                    &as_stated(axis, m(0.0)),
                );
            }
            for tolerance in [50.0, 10.0] {
                parity(
                    fixture,
                    vec![
                        ("axis", string("forward")),
                        target(),
                        ("tolerance", millimetres(tolerance)),
                    ],
                    &as_stated("forward", quantity(tolerance, "mm")),
                );
            }
        }
    }

    #[test]
    fn the_extent_within_a_range_reaches_the_verdicts() {
        for fixture in [walls as Fixture, straddling, unreadable] {
            parity(
                fixture,
                vec![
                    ("axis", string("forward")),
                    ("minimum", length(0.26)),
                    ("maximum", millimetres(400.0)),
                ],
                &between(mm(extent("forward")), m(0.26), quantity(400.0, "mm")),
            );
            parity(
                fixture,
                vec![("axis", string("forward")), ("minimum", length(0.3))],
                &at_least(mm(extent("forward")), m(0.3)),
            );
            parity(
                fixture,
                vec![("axis", string("right")), ("maximum", length(4.0))],
                &at_most(mm(extent("right")), m(4.0)),
            );
        }
    }

    /// Generated walls: boxes of random size turned to a random heading,
    /// measured exactly or within a slack, stating a thickness or none,
    /// judged against the stated thickness within a tolerance or against a
    /// range. The rewrite reaches the capability's verdicts and evidence
    /// exactness on every one.
    mod generated {
        use super::*;
        use axioval_rules::parity::Parity;
        use proptest::collection::vec;
        use proptest::prelude::*;

        /// One wall: its size in mm, its heading in degrees, the slack of
        /// its measurement in mm, and the thickness it states in mm.
        type Wall = ([u32; 3], u32, u32, Option<u32>);

        fn wall() -> impl Strategy<Value = Wall> {
            (
                [1000u32..8000, 50..600, 2000..4000],
                prop_oneof![Just(0u32), Just(90), 0u32..180],
                prop_oneof![Just(0u32), 1u32..20],
                proptest::option::of(50u32..600),
            )
        }

        fn fixture(walls: &[Wall]) -> (Model, Frames, Boxes) {
            let mut model = Model::default();
            let mut frames = Frames::new();
            let mut boxes = Boxes::default();
            for (index, (size, heading, slack, stated)) in walls.iter().enumerate() {
                let local = format!("w{index}");
                model = model.object(&local, "wall");
                if let Some(stated) = stated {
                    model = model.value(
                        &local,
                        MATERIAL,
                        "TotalThickness",
                        metres(f64::from(*stated) / 1000.0),
                    );
                }
                let angle = f64::from(*heading).to_radians();
                frames = frames.with(&local, [angle.cos(), angle.sin(), 0.0]);
                boxes = boxes.with(
                    &local,
                    size.map(|value| f64::from(value) / 1000.0),
                    f64::from(*slack) / 1000.0,
                );
            }
            (model, frames, boxes)
        }

        fn hold(
            walls: &[Wall],
            parameters: Vec<(&str, ParameterValue)>,
            requirement: &Value,
        ) -> Result<(), TestCaseError> {
            let (model, frames, boxes) = fixture(walls);
            let capability = run(model, frames, boxes, parameters);
            let rewrite = fixture(walls).0.evaluate_measured(
                &axioval_rules::ExpressionRequirement,
                &expression(kind("wall"), requirement),
                |services| {
                    let (_, frames, boxes) = fixture(walls);
                    services
                        .register(ObjectFrameServiceHandle::new(Arc::new(frames)))
                        .unwrap();
                    services
                        .register(VerticalExtentServiceHandle::new(Arc::new(boxes)))
                        .unwrap();
                },
            );
            let parity =
                Parity::outcomes().compare_evaluations((ID, &capability), ("expression", &rewrite));
            prop_assert!(parity.holds(), "{}", parity.diff());
            Ok(())
        }

        proptest! {
            #![proptest_config(ProptestConfig {
                cases: 64,
                failure_persistence: None,
                ..ProptestConfig::default()
            })]

            #[test]
            fn generated_walls_against_their_thickness_hold_parity(
                walls in vec(wall(), 1..5),
                tolerance in prop_oneof![Just(0u32), 1u32..60],
            ) {
                let tolerance = f64::from(tolerance);
                hold(
                    &walls,
                    vec![
                        ("axis", string("forward")),
                        ("target_property", thickness()),
                        ("tolerance", millimetres(tolerance)),
                    ],
                    &as_stated("forward", quantity(tolerance, "mm")),
                )?;
            }

            #[test]
            fn generated_walls_within_a_range_hold_parity(
                walls in vec(wall(), 1..5),
                minimum in 50u32..400,
                spread in 0u32..300,
            ) {
                let (low, high) = (f64::from(minimum), f64::from(minimum + spread));
                hold(
                    &walls,
                    vec![
                        ("axis", string("forward")),
                        ("minimum", millimetres(low)),
                        ("maximum", millimetres(high)),
                    ],
                    &between(
                        mm(extent("forward")),
                        quantity(low, "mm"),
                        quantity(high, "mm"),
                    ),
                )?;
            }
        }
    }
}
