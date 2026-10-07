//! A round column against a wall, judged through the capabilities.
//!
//! The column (radius 0.2 m, axis 1 m from the wall's face) is 0.8 m from
//! the wall. Its chord mesh leaves the distance within a few millimetres, so
//! a bound of 0.799 m or 0.8005 m lies inside the mesh's interval and cannot
//! be judged. With both exact boundaries registered the certified boundary
//! distance clears either bound, and `distance` and `clash` decide.
//!
//! In plan the same holds for two columns at different heights whose axes
//! stand 1 m apart: `distance` with `projection: horizontal` judges their
//! certified plan distance of 0.6 m against bounds the chords leave open.
#![cfg(feature = "axiolid")]
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};
use std::sync::Arc;

use axiolid_construct::ExactBRep;
use axiolid_construct::boolean_exact::{ArcPrism, boolean_arc_prisms_exact};
use axiolid_core::{BooleanOperator, Point2, Point3, Tolerance};
use axiolid_mesh::TriMesh;
use axiolid_overlay::ArcRing;
use axioval::axiolid::{AxiolidGeometry, AxiolidProximityService};
use axioval::engine::{
    CapabilityEvaluation, CompiledRule, ProximityServiceHandle, RuleCapability, RuleContext,
    ServiceRegistry,
};
use axioval::ir::contract::{ParameterValue, Selector, Severity};
use axioval::ir::{Object, ObjectId, Project, RuleId, SourceId};
use axioval::rules::{Clash, Distance};

const RADIUS: f64 = 0.2;
const SIDES: u32 = 16;
const HEIGHT: f64 = 3.0;

fn id(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("cad", "model").unwrap(), local).unwrap()
}

fn exact_prism(section: impl Fn(f64) -> ArcRing) -> ExactBRep {
    exact_prism_between(section, [0.0, HEIGHT])
}

fn exact_prism_between(section: impl Fn(f64) -> ArcRing, [bottom, top]: [f64; 2]) -> ExactBRep {
    let prism = |scale: f64| ArcPrism {
        section: section(scale),
        bottom,
        top,
    };
    boolean_arc_prisms_exact(
        &prism(1.0),
        &prism(2.0),
        BooleanOperator::Intersection,
        Tolerance::METRE,
    )
    .expect("an exact prism")
}

/// A closed, outward-oriented axis-aligned box.
fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    TriMesh::new(
        vec![
            Point3::new(x0, y0, z0),
            Point3::new(x1, y0, z0),
            Point3::new(x1, y1, z0),
            Point3::new(x0, y1, z0),
            Point3::new(x0, y0, z1),
            Point3::new(x1, y0, z1),
            Point3::new(x1, y1, z1),
            Point3::new(x0, y1, z1),
        ],
        vec![
            0, 2, 1, 0, 3, 2, // bottom
            4, 5, 6, 4, 6, 7, // top
            0, 1, 5, 0, 5, 4, // front
            3, 7, 6, 3, 6, 2, // back
            0, 4, 7, 0, 7, 3, // left
            1, 2, 6, 1, 6, 5, // right
        ],
    )
}

/// The column's chords, a vertex facing the wall.
fn column_mesh() -> TriMesh {
    column_mesh_at(0.0, [0.0, HEIGHT])
}

/// A column's chords with its axis at `x` on the x axis, between `levels`,
/// vertices at angles zero and π.
fn column_mesh_at(x: f64, levels: [f64; 2]) -> TriMesh {
    let mut positions = Vec::new();
    for level in levels {
        for side in 0..SIDES {
            let angle = TAU * f64::from(side) / f64::from(SIDES);
            positions.push(Point3::new(
                x + RADIUS * angle.cos(),
                RADIUS * angle.sin(),
                level,
            ));
        }
    }
    positions.push(Point3::new(x, 0.0, levels[0]));
    positions.push(Point3::new(x, 0.0, levels[1]));
    let (bottom_centre, top_centre) = (2 * SIDES, 2 * SIDES + 1);
    let mut indices = Vec::new();
    for side in 0..SIDES {
        let next = (side + 1) % SIDES;
        let (b0, b1, t0, t1) = (side, next, side + SIDES, next + SIDES);
        indices.extend([b0, b1, t1, b0, t1, t0]);
        indices.extend([bottom_centre, b1, b0]);
        indices.extend([top_centre, t0, t1]);
    }
    TriMesh::new(positions, indices)
}

fn chord_deviation() -> f64 {
    RADIUS * (1.0 - (PI / f64::from(SIDES)).cos())
}

fn geometry(certified: bool) -> AxiolidGeometry {
    let meshes = AxiolidGeometry::new()
        .with_tessellated_mesh(id("column"), column_mesh(), chord_deviation())
        .with_mesh(id("wall"), cuboid([1.0, -2.0, 0.0], [1.2, 2.0, HEIGHT]));
    if !certified {
        return meshes;
    }
    let corners = [(1.0, -2.0), (1.2, -2.0), (1.2, 2.0), (1.0, 2.0)];
    meshes
        .with_exact_boundary(
            id("column"),
            exact_prism(|scale| ArcRing::circle(Point2::ZERO, RADIUS * scale)),
        )
        .with_exact_boundary(
            id("wall"),
            exact_prism(|scale| {
                ArcRing::from_points(
                    &corners
                        .iter()
                        .map(|(x, y)| Point2::new(1.1 + (x - 1.1) * scale, y * scale))
                        .collect::<Vec<_>>(),
                )
            }),
        )
}

/// Two columns whose axes stand 1 m apart in plan, `low` from 0 to 3 m and
/// `high` from 4 to 6 m: 0.6 m apart in plan, farther in space.
fn two_columns(certified: bool) -> AxiolidGeometry {
    let meshes = AxiolidGeometry::new()
        .with_tessellated_mesh(
            id("low"),
            column_mesh_at(0.0, [0.0, HEIGHT]),
            chord_deviation(),
        )
        .with_tessellated_mesh(
            id("high"),
            column_mesh_at(1.0, [4.0, 6.0]),
            chord_deviation(),
        );
    if !certified {
        return meshes;
    }
    let circle = |x: f64| move |scale: f64| ArcRing::circle(Point2::new(x, 0.0), RADIUS * scale);
    meshes
        .with_exact_boundary(id("low"), exact_prism(circle(0.0)))
        .with_exact_boundary(id("high"), exact_prism_between(circle(1.0), [4.0, 6.0]))
}

fn check(
    capability: &dyn RuleCapability,
    certified: bool,
    parameters: &[(&str, f64)],
) -> CapabilityEvaluation {
    let numbers = parameters
        .iter()
        .map(|(name, value)| (*name, ParameterValue::Number { value: *value }));
    evaluate(
        capability,
        geometry(certified),
        ["column", "wall"],
        numbers.collect(),
    )
}

/// Evaluates `capability` for objects of the first kind against the second,
/// each object named and typed by its kind.
fn evaluate(
    capability: &dyn RuleCapability,
    geometry: AxiolidGeometry,
    [subject, counterpart]: [&str; 2],
    parameters: Vec<(&str, ParameterValue)>,
) -> CapabilityEvaluation {
    let mut bound = BTreeMap::from([(
        "counterparts".to_owned(),
        ParameterValue::Selector {
            value: Box::new(kind(counterpart)),
        },
    )]);
    for (name, value) in parameters {
        bound.insert(name.to_owned(), value);
    }
    let rule = CompiledRule {
        id: RuleId::new("check").unwrap(),
        capability: capability.id().into(),
        severity: Severity::Error,
        selector: kind(subject),
        parameters: bound,
    };
    let project = Project::new(vec![
        Object::new(id(subject), subject),
        Object::new(id(counterpart), counterpart),
    ])
    .unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(ProximityServiceHandle::new(Arc::new(
            AxiolidProximityService::new(geometry),
        )))
        .unwrap();
    // A template reads its measured lists as a run does.
    let registry =
        axioval::rules::register_builtins(axioval::engine::CapabilityRegistry::new()).unwrap();
    registry.install_measured(&mut services, &project);
    let context = RuleContext {
        project: &project,
        services: &services,
    };
    let evaluated = capability.evaluate(&context, &rule);
    // `distance` is held to the implementation it replaced, on the same
    // geometry.
    if capability.id() == "axioval:capability.distance" {
        let replaced = axioval_rules::reference::Distance.evaluate(&context, &rule);
        let parity = axioval::rules::parity::Parity::contract().compare(
            (
                capability.id(),
                &axioval::rules::parity::Observations::of_evaluation(&replaced),
            ),
            (
                "template",
                &axioval::rules::parity::Observations::of_evaluation(&evaluated),
            ),
        );
        assert!(parity.holds(), "{}", parity.diff());
    }
    evaluated
}

fn kind(kind: &str) -> Selector {
    Selector::EntityType {
        object_type: kind.into(),
        include_subtypes: false,
    }
}

fn decided(outcome: &CapabilityEvaluation) -> bool {
    outcome.not_evaluated_outcomes().is_empty()
}

#[test]
fn a_round_column_clears_a_minimum_distance_to_a_wall() {
    let minimum = [("minimum_metres", 0.799)];
    let chords = check(&Distance, false, &minimum);
    assert!(chords.findings().is_empty());
    assert!(!decided(&chords), "the chords alone cannot judge 0.799 m");

    let certified = check(&Distance, true, &minimum);
    assert!(certified.findings().is_empty());
    assert!(
        decided(&certified),
        "{:?}",
        certified.not_evaluated_outcomes()
    );
}

#[test]
fn a_round_column_breaks_a_clearance_it_only_just_misses() {
    let clearance = [
        ("penetration_tolerance_metres", 0.0),
        ("clearance_metres", 0.8005),
    ];
    let certified = check(&Clash, true, &clearance);
    let [finding] = certified.findings() else {
        panic!("one clearance clash expected: {certified:?}");
    };
    assert!(
        finding.message.starts_with("clearance clash"),
        "{}",
        finding.message
    );
    assert!(finding.message.contains("certified"), "{}", finding.message);
    assert!(!finding.evidence[0].exact, "the pair is still tessellated");

    let distance = check(&Distance, true, &[("minimum_metres", 0.8005)]);
    let [finding] = distance.findings() else {
        panic!("one distance finding expected: {distance:?}");
    };
    assert!(
        finding.message.contains("closer than"),
        "{}",
        finding.message
    );
    let chords = check(&Distance, false, &[("minimum_metres", 0.8005)]);
    assert!(chords.findings().is_empty() && !decided(&chords));
}

#[test]
fn two_round_columns_are_judged_by_their_certified_plan_distance() {
    // 0.6 m apart in plan; the chords leave 0.5995 m and 0.6005 m open.
    let horizontal = |minimum: f64| {
        vec![
            (
                "projection",
                ParameterValue::String {
                    value: "horizontal".into(),
                },
            ),
            ("minimum_metres", ParameterValue::Number { value: minimum }),
        ]
    };
    for minimum in [0.5995, 0.6005] {
        let chords = evaluate(
            &Distance,
            two_columns(false),
            ["low", "high"],
            horizontal(minimum),
        );
        assert!(
            chords.findings().is_empty() && !decided(&chords),
            "{minimum}"
        );
    }

    let clear = evaluate(
        &Distance,
        two_columns(true),
        ["low", "high"],
        horizontal(0.5995),
    );
    assert!(clear.findings().is_empty(), "{clear:?}");
    assert!(decided(&clear), "{:?}", clear.not_evaluated_outcomes());

    let close = evaluate(
        &Distance,
        two_columns(true),
        ["low", "high"],
        horizontal(0.6005),
    );
    let [finding] = close.findings() else {
        panic!("one distance finding expected: {close:?}");
    };
    assert!(
        finding.message.contains("closer than"),
        "{}",
        finding.message
    );
    assert!(!finding.evidence[0].exact, "the pair is still tessellated");
}
