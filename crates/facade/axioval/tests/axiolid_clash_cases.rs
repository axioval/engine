//! Clash tolerance cases along the elements' own axes, on real meshes.
//!
//! A slab edge sunk 10 mm into a wall standing at 30°: the Axiolid
//! proximity service measures the intersection along the wall's placement
//! axes, which a small object-frame service states.
#![cfg(feature = "axiolid")]
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval::axiolid::{AxiolidGeometry, AxiolidProximityService};
use axioval::engine::{
    CapabilityEvaluation, CompiledRule, MetricDirection, MetricFrame, MetricPoint, ObjectFrame,
    ObjectFrameError, ObjectFrameService, ObjectFrameServiceHandle, ObjectFront,
    ProximityServiceHandle, RuleCapability, RuleContext, ServiceRegistry, SourceSnapshot,
};
use axioval::ir::contract::{ParameterValue, Selector, Severity, TableRow};
use axioval::ir::{Evidence, Object, ObjectId, Project, RuleId, SourceId};
use axioval::rules::Clash;

const ANGLE: f64 = std::f64::consts::PI / 6.0;

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

/// A closed, outward-oriented box of `min`..`max` in its own frame, turned
/// by [`ANGLE`] about the vertical axis through the origin.
fn turned_box(min: [f64; 3], max: [f64; 3]) -> TriMesh {
    let (sin, cos) = ANGLE.sin_cos();
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let corners = [
        [x0, y0, z0],
        [x1, y0, z0],
        [x1, y1, z0],
        [x0, y1, z0],
        [x0, y0, z1],
        [x1, y0, z1],
        [x1, y1, z1],
        [x0, y1, z1],
    ];
    TriMesh::new(
        corners
            .iter()
            .map(|[u, v, z]| Point3::new(u * cos - v * sin, u * sin + v * cos, *z))
            .collect(),
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

/// Both bodies are placed along the wall: right along its length, forward
/// across it, up.
struct Frames(Vec<SourceSnapshot>);

impl ObjectFrameService for Frames {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.0
    }

    fn object_frame(&self, object: &ObjectId) -> Result<ObjectFrame, ObjectFrameError> {
        let (sin, cos) = ANGLE.sin_cos();
        let axis = |vector| MetricDirection::try_new(vector).unwrap();
        ObjectFrame::try_new(
            object.clone(),
            MetricFrame::try_new(
                MetricPoint::try_new(object.clone(), [0.0; 3]).unwrap(),
                axis([cos, sin, 0.0]),
                axis([-sin, cos, 0.0]),
                axis([0.0, 0.0, 1.0]),
            )
            .unwrap(),
            ObjectFront::NotStated,
            Evidence::exact(source(), format!("placement:{}", object.local_id)),
        )
    }
}

fn kind(kind: &str) -> Selector {
    Selector::EntityType {
        object_type: kind.into(),
        include_subtypes: false,
    }
}

fn check(cases: Vec<TableRow>) -> CapabilityEvaluation {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), turned_box([-2.0, -0.1, 0.0], [2.0, 0.1, 3.0]))
        .with_mesh(id("slab"), turned_box([-1.5, 0.09, 1.0], [1.5, 3.09, 1.2]));
    let project = Project::new(vec![
        Object::new(id("wall"), "wall"),
        Object::new(id("slab"), "slab"),
    ])
    .unwrap();
    let mut parameters = BTreeMap::from([
        (
            "counterparts".to_owned(),
            ParameterValue::Selector {
                value: Box::new(kind("wall")),
            },
        ),
        (
            "penetration_tolerance_metres".to_owned(),
            ParameterValue::Number { value: 0.0 },
        ),
    ]);
    if !cases.is_empty() {
        parameters.insert(
            "tolerance_cases".to_owned(),
            ParameterValue::Table { value: cases },
        );
    }
    let rule = CompiledRule {
        id: RuleId::new("clash").unwrap(),
        capability: "axioval:capability.clash".into(),
        severity: Severity::Error,
        selector: kind("slab"),
        parameters,
    };
    let mut services = ServiceRegistry::new();
    services
        .register(ProximityServiceHandle::new(Arc::new(
            AxiolidProximityService::new(geometry),
        )))
        .unwrap();
    services
        .register(ObjectFrameServiceHandle::new(Arc::new(Frames(vec![
            SourceSnapshot::try_new(source(), "r1", "sha256:1").unwrap(),
        ]))))
        .unwrap();
    Clash.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        &rule,
    )
}

fn orthogonal(tolerance: f64) -> TableRow {
    [
        (
            "case",
            ParameterValue::String {
                value: "horizontal_orthogonal".into(),
            },
        ),
        (
            "first_selector",
            ParameterValue::Selector {
                value: Box::new(kind("slab")),
            },
        ),
        (
            "second_selector",
            ParameterValue::Selector {
                value: Box::new(kind("wall")),
            },
        ),
        (
            "tolerance_metres",
            ParameterValue::Number { value: tolerance },
        ),
    ]
    .into_iter()
    .map(|(column, value)| (column.to_owned(), value))
    .collect()
}

/// A 30° wall with a slab edge sunk 10 mm into it passes a 20 mm
/// orthogonal case, and fails without it or with a 5 mm one.
#[test]
fn a_slab_edge_in_a_turned_wall_passes_an_orthogonal_case() {
    let excused = check(vec![orthogonal(0.02)]);
    assert!(
        excused.findings().is_empty() && excused.not_evaluated_outcomes().is_empty(),
        "{:?} {:?}",
        excused.findings(),
        excused.not_evaluated_outcomes()
    );

    for cases in [vec![], vec![orthogonal(0.005)]] {
        let found = check(cases);
        assert!(found.not_evaluated_outcomes().is_empty());
        let [finding] = found.findings() else {
            panic!("one clash expected: {:?}", found.findings());
        };
        assert!(
            finding.message.starts_with("hard clash with"),
            "{}",
            finding.message
        );
    }
}
