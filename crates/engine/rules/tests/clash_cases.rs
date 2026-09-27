//! Special-case clash tolerances along the elements' own axes.
//!
//! A slab edge sunk 10 mm into a wall standing at 30°: along the world axes
//! the intersection reaches metres, along the wall's own thickness 10 mm.
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use axioval_engine::{
    Bounds3, CapabilityEvaluation, CompiledRule, GeometryFidelity, LengthInterval, MetricDirection,
    MetricFrame, MetricPoint, ObjectBounds, ObjectFrame, ObjectFrameError, ObjectFrameService,
    ObjectFrameServiceHandle, ObjectFront, OverlapAlongEvidence, OverlapAlongRequest,
    OverlapExtents, ProximityError, ProximityEvidence, ProximityRequest, ProximityService,
    ProximityServiceHandle, SourceSnapshot,
};
use axioval_ir::contract::{ParameterValue, TableRow};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId};
use axioval_rules::{Clash, ClashMatrix};

use common::{Model, kind, number, rule, selector, source, string, unevaluated};

const ANGLE: f64 = std::f64::consts::PI / 6.0;

/// The wall's axes: along its length, across its thickness, up.
fn wall_axes() -> [[f64; 3]; 3] {
    let (sin, cos) = ANGLE.sin_cos();
    [[cos, sin, 0.0], [-sin, cos, 0.0], [0.0, 0.0, 1.0]]
}

/// The slab's axes: the world's.
fn slab_axes() -> [[f64; 3]; 3] {
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
}

/// The intersection's extents along the wall's axes and the slab's:
/// `across` through the wall's thickness, 3 m along it, 0.2 m high, and
/// 2.6 m and 1.5 m along the world's plan axes.
fn extent_along(direction: [f64; 3], across: (f64, f64)) -> (f64, f64) {
    let [along, thickness, up] = wall_axes();
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    if (dot(direction, thickness).abs() - 1.0).abs() < 1e-9 {
        across
    } else if (dot(direction, along).abs() - 1.0).abs() < 1e-9 {
        (3.0, 3.0)
    } else if (dot(direction, up).abs() - 1.0).abs() < 1e-9 {
        (0.2, 0.2)
    } else if (direction[0].abs() - 1.0).abs() < 1e-9 {
        (2.6, 2.6)
    } else if (direction[1].abs() - 1.0).abs() < 1e-9 {
        (1.5, 1.5)
    } else {
        panic!("unexpected direction {direction:?}")
    }
}

/// A slab sunk into a wall, measured along any axes asked for.
struct Stub {
    /// The intersection's extent through the wall's thickness; `None`
    /// refuses the measurement along axes.
    across: Option<(f64, f64)>,
}

impl ProximityService for Stub {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        ObjectBounds::try_new(
            object.clone(),
            Bounds3::try_new([0.0; 3], [3.0, 3.0, 3.0])?,
            GeometryFidelity::Exact,
        )
    }

    fn measure_proximity(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProximityEvidence, ProximityError> {
        let world = |extent: f64| LengthInterval::exact(extent).unwrap();
        ProximityEvidence::try_new(
            request.clone(),
            0.0,
            Some(0.01),
            0.0,
            None,
            GeometryFidelity::Exact,
            Evidence::exact(source(), "proximity"),
        )?
        .with_hausdorff(LengthInterval::try_new(1.0, 1.0).unwrap())?
        .with_overlap_extents(OverlapExtents::new(world(2.6), world(1.5), world(0.2)))
    }

    fn measure_overlap_along(
        &self,
        request: &OverlapAlongRequest,
    ) -> Result<OverlapAlongEvidence, ProximityError> {
        let across = self.across.ok_or(ProximityError::Unavailable)?;
        let extents = request
            .directions()
            .iter()
            .map(|direction| {
                let (lower, upper) = extent_along(direction.components(), across);
                LengthInterval::try_new(lower, upper).unwrap()
            })
            .collect();
        OverlapAlongEvidence::try_new(
            request.clone(),
            extents,
            GeometryFidelity::Exact,
            Evidence::exact(source(), "along"),
        )
    }
}

struct Frames(Vec<SourceSnapshot>);

impl ObjectFrameService for Frames {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.0
    }

    fn object_frame(&self, object: &ObjectId) -> Result<ObjectFrame, ObjectFrameError> {
        let axes = match object.local_id.as_str() {
            "wall" => wall_axes(),
            "slab" => slab_axes(),
            _ => return Err(ObjectFrameError::NotPlaced(object.clone())),
        };
        let [right, forward, up] = axes.map(|axis| MetricDirection::try_new(axis).unwrap());
        ObjectFrame::try_new(
            object.clone(),
            MetricFrame::try_new(
                MetricPoint::try_new(object.clone(), [0.0; 3]).unwrap(),
                right,
                forward,
                up,
            )
            .unwrap(),
            ObjectFront::NotStated,
            Evidence::exact(source(), format!("placement:{}", object.local_id)),
        )
    }
}

fn model() -> Model {
    Model::default()
        .object("slab", "slab")
        .object("wall", "wall")
}

fn case(name: &str, first: &str, second: &str, tolerance: f64) -> TableRow {
    [
        ("case", string(name)),
        ("first_selector", selector(kind(first))),
        ("second_selector", selector(kind(second))),
        ("tolerance_metres", number(tolerance)),
    ]
    .into_iter()
    .map(|(column, value)| (column.to_owned(), value))
    .collect()
}

fn clash(cases: Vec<TableRow>) -> CompiledRule {
    let mut parameters = vec![
        ("counterparts", selector(kind("wall"))),
        ("penetration_tolerance_metres", number(0.0)),
    ];
    if !cases.is_empty() {
        parameters.push(("tolerance_cases", ParameterValue::Table { value: cases }));
    }
    rule("axioval:capability.clash", kind("slab"), parameters)
}

fn run_with(
    capability: &dyn axioval_engine::RuleCapability,
    stub: Stub,
    frames: bool,
    rule: &CompiledRule,
) -> CapabilityEvaluation {
    model().evaluate_with(capability, rule, |services| {
        services
            .register(ProximityServiceHandle::new(Arc::new(stub)))
            .unwrap();
        if frames {
            services
                .register(ObjectFrameServiceHandle::new(Arc::new(Frames(vec![
                    SourceSnapshot::try_new(source(), "r1", "sha256:1").unwrap(),
                ]))))
                .unwrap();
        }
    })
}

fn run(across: (f64, f64), rule: &CompiledRule) -> CapabilityEvaluation {
    run_with(
        &Clash,
        Stub {
            across: Some(across),
        },
        true,
        rule,
    )
}

fn passes(evaluation: &CapabilityEvaluation) -> bool {
    evaluation.findings().is_empty() && evaluation.not_evaluated_outcomes().is_empty()
}

/// A slab edge sunk 10 mm into a wall at 30° passes a 20 mm orthogonal
/// case, and fails without it.
#[test]
fn an_orthogonal_case_measures_across_the_second_elements_axes() {
    let sunk = (0.01, 0.01);
    let excused = run(
        sunk,
        &clash(vec![case("horizontal_orthogonal", "slab", "wall", 0.02)]),
    );
    assert!(passes(&excused), "{:?}", excused.not_evaluated_outcomes());

    let evaluation = run(sunk, &clash(vec![]));
    assert_eq!(evaluation.findings().len(), 1);

    // A tighter case does not excuse it, and the finding cites the axes.
    let evaluation = run(
        sunk,
        &clash(vec![case("horizontal_orthogonal", "slab", "wall", 0.005)]),
    );
    let [finding] = evaluation.findings() else {
        panic!(
            "one finding expected: {:?}",
            evaluation.not_evaluated_outcomes()
        );
    };
    assert!(
        finding
            .evidence
            .iter()
            .any(|evidence| evidence.locator == "along")
    );
}

/// The filters name the first and the second element, and the case whose
/// axes are measured: along the slab's own axes the intersection reaches
/// metres.
#[test]
fn the_filters_choose_whose_axes_are_measured() {
    let evaluation = run(
        (0.01, 0.01),
        &clash(vec![case("horizontal_protrusion", "wall", "slab", 0.02)]),
    );
    assert!(passes(&evaluation));
    for (name, first, second) in [
        ("horizontal_protrusion", "slab", "wall"),
        ("horizontal_orthogonal", "wall", "slab"),
    ] {
        let evaluation = run((0.01, 0.01), &clash(vec![case(name, first, second, 0.02)]));
        assert_eq!(evaluation.findings().len(), 1, "{name} {first} {second}");
    }

    let evaluation = run(
        (0.01, 0.01),
        &clash(vec![case("vertical_orthogonal", "slab", "wall", 0.02)]),
    );
    assert_eq!(evaluation.findings().len(), 1, "0.2 m high, not 20 mm");

    let evaluation = run(
        (0.01, 0.01),
        &clash(vec![case("vertical_protrusion", "slab", "wall", 0.3)]),
    );
    assert!(passes(&evaluation));

    // A case for other elements does not apply.
    let evaluation = run(
        (0.01, 0.01),
        &clash(vec![case("horizontal_orthogonal", "beam", "wall", 0.02)]),
    );
    assert_eq!(evaluation.findings().len(), 1);
}

/// An extent straddling the tolerance, a missing frame or a refused
/// measurement leaves the pair open, never excused and never reported.
#[test]
fn an_undecided_case_leaves_the_pair_open() {
    let rule = clash(vec![case("horizontal_orthogonal", "slab", "wall", 0.02)]);
    let straddling = run((0.015, 0.025), &rule);
    assert!(straddling.findings().is_empty());
    let [open] = straddling.not_evaluated_outcomes() else {
        panic!("one open pair expected");
    };
    assert!(
        open.message()
            .contains("horizontal orthogonal case 0 (0.0200 m)"),
        "{}",
        open.message()
    );

    let without_frames = run_with(
        &Clash,
        Stub {
            across: Some((0.01, 0.01)),
        },
        false,
        &rule,
    );
    assert!(without_frames.findings().is_empty());
    assert!(
        without_frames.not_evaluated_outcomes()[0]
            .message()
            .contains("object-frame service is not registered"),
        "{:?}",
        without_frames.not_evaluated_outcomes()
    );

    let refused = run_with(&Clash, Stub { across: None }, true, &rule);
    assert!(refused.findings().is_empty());
    assert_eq!(
        unevaluated(&refused),
        vec![("slab".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn a_matrix_takes_the_cases_rule_wide() {
    let cell: TableRow = [("penetration_tolerance_metres".to_owned(), number(0.0))]
        .into_iter()
        .collect();
    let matrix = |cases: Vec<TableRow>| {
        let mut parameters = vec![
            ("counterparts", selector(kind("wall"))),
            (
                "cells",
                ParameterValue::Table {
                    value: vec![cell.clone()],
                },
            ),
            ("exclude_same_system", common::boolean(false)),
        ];
        if !cases.is_empty() {
            parameters.push(("tolerance_cases", ParameterValue::Table { value: cases }));
        }
        rule("axioval:capability.clash-matrix", kind("slab"), parameters)
    };
    let stub = || Stub {
        across: Some((0.01, 0.01)),
    };
    let excused = run_with(
        &ClashMatrix,
        stub(),
        true,
        &matrix(vec![case("horizontal_orthogonal", "slab", "wall", 0.02)]),
    );
    assert!(passes(&excused));
    let found = run_with(&ClashMatrix, stub(), true, &matrix(vec![]));
    assert_eq!(found.findings().len(), 1);
}

#[test]
fn invalid_cases_refuse_the_rule() {
    let mut named = case("diagonal", "slab", "wall", 0.02);
    for rows in [vec![named.clone()], {
        named.insert("case".into(), string("horizontal_orthogonal"));
        named.insert("tolerance_metres".into(), number(-0.01));
        vec![named.clone()]
    }] {
        let evaluation = run((0.01, 0.01), &clash(rows));
        assert_eq!(
            unevaluated(&evaluation),
            vec![("slab".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
    // The selectors are optional: a case without them covers every pair.
    let mut open = case("horizontal_orthogonal", "slab", "wall", 0.02);
    open.remove("first_selector");
    open.remove("second_selector");
    assert!(passes(&run((0.01, 0.01), &clash(vec![open]))));
}
