//! Slopes, falls and tilts read as `axioval:measured` values by expression
//! rules: angles over the certified normals of a face, or the axes of a
//! placement.
#![allow(missing_docs, clippy::needless_pass_by_value)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, CoordinateSystemError, CoordinateSystemService,
    CoordinateSystemServiceHandle, EvidenceSession, FaceNormal, FaceNormals, MetricDirection,
    MetricFrame, MetricPoint, ObjectFrame, ObjectFrameError, ObjectFrameService,
    ObjectFrameServiceHandle, ObjectFront, SourceCoordinateSystem, SurfaceFace, VerticalExtent,
    VerticalExtentError, VerticalExtentService, VerticalExtentServiceHandle,
};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, Report};
use axioval_rules::register_builtins;
use common::runtime::{definitions, entity, plan, rule, run, session, snapshot};
use common::{Model, id, source};
use serde_json::{Value, json};

const EXPRESSION: &str = "axioval:capability.expression";

/// The top face of each object, as normals.
struct Faces(BTreeMap<ObjectId, Vec<FaceNormal>>);

impl VerticalExtentService for Faces {
    fn measure_vertical_extent(&self, _: &ObjectId) -> Result<VerticalExtent, VerticalExtentError> {
        Err(VerticalExtentError::Unavailable("faces only".into()))
    }

    fn measure_face_normals(
        &self,
        object: &ObjectId,
        face: SurfaceFace,
    ) -> Result<FaceNormals, VerticalExtentError> {
        let normals = self
            .0
            .get(object)
            .cloned()
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?;
        let mut evidence = Evidence::exact(source(), format!("faces:{object}"));
        evidence.exact = normals.iter().all(FaceNormal::is_exact);
        FaceNormals::try_new(object.clone(), face, normals, evidence)
    }
}

/// Each object placed at the origin, its own x axis turned `angle` in plan
/// and tilted `tilt` about it.
struct Frames(
    BTreeMap<ObjectId, (f64, f64)>,
    Vec<axioval_engine::SourceSnapshot>,
);

impl ObjectFrameService for Frames {
    fn source_snapshots(&self) -> &[axioval_engine::SourceSnapshot] {
        &self.1
    }

    fn object_frame(&self, object: &ObjectId) -> Result<ObjectFrame, ObjectFrameError> {
        let (angle, tilt) = *self
            .0
            .get(object)
            .ok_or_else(|| ObjectFrameError::Unreadable("unplaced".into()))?;
        let (sin, cos) = angle.sin_cos();
        let (tilt_sin, tilt_cos) = tilt.sin_cos();
        let right = [cos, sin, 0.0];
        let forward = [-sin * tilt_cos, cos * tilt_cos, tilt_sin];
        let up = [sin * tilt_sin, -cos * tilt_sin, tilt_cos];
        let frame = MetricFrame::try_new(
            MetricPoint::try_new(object.clone(), [0.0; 3]).unwrap(),
            MetricDirection::try_new(right).unwrap(),
            MetricDirection::try_new(forward).unwrap(),
            MetricDirection::try_new(up).unwrap(),
        )
        .unwrap();
        ObjectFrame::try_new(
            object.clone(),
            frame,
            ObjectFront::NotStated,
            Evidence::exact(source(), format!("frame:{object}")),
        )
    }
}

fn exact(vector: [f64; 3]) -> FaceNormal {
    FaceNormal::exact(vector).unwrap()
}

fn slabs() -> EvidenceSession {
    let model = ["flat", "gentle", "steep", "warped", "wall"]
        .iter()
        .fold(Model::default(), |model, local| model.object(local, "slab"))
        .edge("over", "steep", "flat")
        .edge("over", "gentle", "flat");
    let faces = Faces(BTreeMap::from([
        (id("flat"), vec![exact([0.0, 0.0, 1.0])]),
        // One in twenty, falling towards +x.
        (id("gentle"), vec![exact([0.05, 0.0, 1.0])]),
        // One in eight, rising towards +y.
        (id("steep"), vec![exact([0.0, -0.125, 1.0])]),
        // Level to one in eight: straddles six percent.
        (
            id("warped"),
            vec![exact([0.0, 0.0, 1.0]), exact([0.125, 0.0, 1.0])],
        ),
        (
            id("wall"),
            vec![FaceNormal::try_new([1.0, 0.0, -0.01], [1.0, 0.0, 0.01]).unwrap()],
        ),
    ]));
    let frames = Frames(
        BTreeMap::from([
            (id("flat"), (0.0, 0.0)),
            (id("gentle"), (std::f64::consts::FRAC_PI_2, 0.0)),
            (id("steep"), (0.0, 0.1)),
        ]),
        vec![snapshot()],
    );
    session(model)
        .with_host_service(
            VerticalExtentServiceHandle::new(Arc::new(faces)),
            &[snapshot()],
        )
        .unwrap()
        .with_host_service(
            ObjectFrameServiceHandle::new(Arc::new(frames)),
            &[snapshot()],
        )
        .unwrap()
        .with_host_service(
            CoordinateSystemServiceHandle::new(Arc::new(TrueNorthEast(vec![snapshot()]))),
            &[snapshot()],
        )
        .unwrap()
}

/// A source whose true north points along its x axis.
struct TrueNorthEast(Vec<axioval_engine::SourceSnapshot>);

impl CoordinateSystemService for TrueNorthEast {
    fn source_snapshots(&self) -> &[axioval_engine::SourceSnapshot] {
        &self.0
    }

    fn coordinate_system(
        &self,
        source: &axioval_ir::SourceId,
    ) -> Result<SourceCoordinateSystem, CoordinateSystemError> {
        SourceCoordinateSystem::try_new(
            source.clone(),
            None,
            Some([1.0, 0.0]),
            None,
            Evidence::exact(source.clone(), "north:east"),
        )
    }
}

fn measured(name: &str) -> Value {
    json!({"kind": "property", "propertySet": "axioval:measured", "property": name})
}

fn percent(name: &str) -> Value {
    json!({"kind": "convertSlope", "operand": measured(name), "from": "angle", "to": "percent"})
}

fn number(value: f64) -> Value {
    json!({"kind": "literal", "value": {"type": "number", "value": value}})
}

fn degrees(value: f64) -> Value {
    json!({"kind": "literal", "value": {"type": "quantity", "value": value, "unit": "deg"}})
}

fn requirement(id: &str, requirement: Value) -> Value {
    rule(
        id,
        EXPRESSION,
        "error",
        entity("slab"),
        json!({"requirement": {"type": "expression", "value": requirement}}),
        json!({}),
    )
}

fn at_most(left: Value, right: Value) -> Value {
    json!({"kind": "compare", "operator": "lessThanOrEquals", "left": left, "right": right})
}

fn check(rules: Vec<Value>) -> Report {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let definitions = definitions(&registry, &[EXPRESSION], &["slab"], &[], &[]);
    let plan = plan(&registry, &definitions, rules).unwrap();
    run(registry, plan, &slabs(), |runtime| runtime).unwrap()
}

fn found(report: &Report, rule: &str) -> Vec<String> {
    let mut found: Vec<String> = report
        .findings()
        .iter()
        .filter(|finding| finding.rule_id.to_string() == rule)
        .map(common::subject)
        .collect();
    found.sort();
    found
}

fn open(report: &Report, rule: &str) -> Vec<(String, NotEvaluatedReason)> {
    let mut open: Vec<(String, NotEvaluatedReason)> = report
        .not_evaluated
        .iter()
        .filter(|outcome| outcome.rule_id.to_string() == rule)
        .filter_map(|outcome| match &outcome.scope {
            axioval_ir::Scope::Object(object) => {
                Some((object.local_id.clone(), outcome.reason.clone()))
            }
            _ => None,
        })
        .collect();
    open.sort();
    open
}

#[test]
fn a_slope_limit_finds_the_steep_face_and_leaves_the_straddling_one_open() {
    let report = check(vec![requirement(
        "slope",
        at_most(percent("slope"), number(6.0)),
    )]);
    assert_eq!(found(&report, "slope"), ["steep"]);
    let open = open(&report, "slope");
    assert_eq!(open.len(), 2, "{open:?}");
    assert_eq!(open[0].0, "wall");
    assert_eq!(open[1].0, "warped");
}

#[test]
fn falls_are_read_along_and_across_axes() {
    let report = check(vec![
        // Falling along +x is a negative gradient: nothing rises along x.
        requirement(
            "along-x",
            at_most(measured("slope_along;direction=x"), degrees(0.0)),
        ),
        // `steep` rises along y.
        requirement(
            "along-y",
            at_most(measured("slope_along;direction=y"), degrees(0.0)),
        ),
        // Across the own x axis: `gentle` is turned a quarter, so its own x
        // runs along y and its fall across it is the full one in twenty.
        requirement(
            "cross",
            at_most(percent("cross_fall;axis=own_x"), number(2.0)),
        ),
    ]);
    assert!(found(&report, "along-x").is_empty());
    assert_eq!(found(&report, "along-y"), ["steep"]);
    assert_eq!(found(&report, "cross"), ["gentle", "steep"]);
}

#[test]
fn the_direction_of_descent_and_the_tilt_of_an_axis_are_angles() {
    let report = check(vec![
        // `gentle` descends towards +x: a bearing of a quarter turn.
        requirement(
            "east",
            json!({"kind": "between",
                "operand": measured("gradient_direction"),
                "low": degrees(89.9), "high": degrees(90.1)}),
        ),
        requirement(
            "plumb",
            at_most(measured("inclination;axis=own_z"), degrees(1.0)),
        ),
    ]);
    // The flat face and a piece of the warped one descend nowhere; the
    // wall stands vertical.
    assert_eq!(found(&report, "east"), ["steep"]);
    let open = open(&report, "east");
    assert_eq!(
        open.iter()
            .map(|(object, _)| object.as_str())
            .collect::<Vec<_>>(),
        ["flat", "wall", "warped"]
    );
    // `steep` is tilted 0.1 rad, about 5.7°.
    assert_eq!(found(&report, "plumb"), ["steep"]);
}

#[test]
fn faces_meet_their_neighbours_at_the_angle_of_their_normals() {
    let angle = measured("angle_to;between=face_normal;path=over");
    let report = check(vec![requirement(
        "pitch",
        json!({"kind": "or", "operands": [
            {"kind": "isUndefined", "operand": angle},
            at_most(angle.clone(), degrees(5.0)),
        ]}),
    )]);
    // `steep` meets `flat` at one in eight (7.1°), `gentle` at one in
    // twenty (2.9°); the others reach nothing over, so have no angle.
    assert_eq!(found(&report, "pitch"), ["steep"]);
    assert!(open(&report, "pitch").is_empty());
}

#[test]
fn bearings_are_read_from_project_or_true_north() {
    let report = check(vec![
        requirement(
            "true",
            json!({"kind": "between",
                "operand": measured("bearing;axis=own_x;reference=true_north"),
                "low": degrees(269.0), "high": degrees(271.0)}),
        ),
        requirement(
            "project",
            at_most(measured("bearing;axis=own_x"), degrees(1.0)),
        ),
    ]);
    // A requirement finds the objects failing it. `gentle` is turned a
    // quarter: its own x runs to project north, a quarter turn anticlockwise
    // of true north (east), so it alone holds both; the others' own x runs
    // east, true north itself and a quarter turn from project north.
    assert_eq!(found(&report, "true"), ["flat", "steep"]);
    assert_eq!(found(&report, "project"), ["flat", "steep"]);
    let open = open(&report, "true");
    assert_eq!(
        open.iter()
            .map(|(object, _)| object.as_str())
            .collect::<Vec<_>>(),
        ["wall", "warped"]
    );
}
