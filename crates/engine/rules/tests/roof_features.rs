//! Roof feature lines (`roof_features`, #329): the lines where a roof's
//! upper faces meet or end, classified from the faces' normals and the
//! side each lies on, with their lengths and heights; a line the geometry
//! cannot classify soundly is undecided, never guessed.
#![allow(missing_docs, clippy::needless_pass_by_value)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, EdgeSide, EvidenceSession, FaceEdge, FaceEdges, FaceNormal, MeasuredMember,
    Measurement, MemberValue, MetricDirection, MetricFrame, MetricPoint, ObjectFrame,
    ObjectFrameError, ObjectFrameService, ObjectFrameServiceHandle, ObjectFront, PointBox,
    SourceSnapshot, VerticalExtent, VerticalExtentError, VerticalExtentService,
    VerticalExtentServiceHandle, measured_members,
};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, Report};
use axioval_rules::register_builtins;
use common::runtime::{definitions, entity, plan, rule, run, session, snapshot};
use common::{Model, id, source};
use serde_json::{Value, json};

const EXPRESSION: &str = "axioval:capability.expression";

type P = [f64; 3];

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: P, b: P) -> P {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// A face beside a line: its normal, and a point on it off the line.
type Face = (P, P);

/// The line from `start` to `end` between `faces`, each known within
/// `slack` (zero: exactly).
fn line(start: P, end: P, faces: &[Face], slack: f64) -> FaceEdge {
    let along = sub(end, start);
    let corner = |at: P| {
        if slack == 0.0 {
            PointBox::exact(at).unwrap()
        } else {
            PointBox::try_new(at.map(|c| c - slack), at.map(|c| c + slack)).unwrap()
        }
    };
    let sides = faces
        .iter()
        .map(|(normal, on)| {
            let left = dot(cross(*normal, along), sub(*on, start)) > 0.0;
            let normal = if slack == 0.0 {
                FaceNormal::exact(*normal).unwrap()
            } else {
                FaceNormal::try_new(normal.map(|c| c - slack), normal.map(|c| c + slack)).unwrap()
            };
            EdgeSide::new(normal, left)
        })
        .collect();
    FaceEdge::try_new(corner(start), corner(end), sides).unwrap()
}

/// Each roof's lines, and whether they are exact.
#[derive(Default)]
struct Roofs(BTreeMap<ObjectId, (Vec<FaceEdge>, bool)>);

impl VerticalExtentService for Roofs {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        Err(VerticalExtentError::UnknownObject(object.clone()))
    }

    fn measure_face_edges(&self, object: &ObjectId) -> Result<FaceEdges, VerticalExtentError> {
        let (edges, exact) = self
            .0
            .get(object)
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?;
        let mut evidence = Evidence::exact(source(), format!("face-edges:{}", object.local_id));
        evidence.exact = *exact;
        FaceEdges::try_new(object.clone(), edges.clone(), evidence)
    }
}

const SOUTH: P = [0.0, -1.0, 1.0];
const NORTH: P = [0.0, 1.0, 1.0];
const DOWN: P = [0.0, 0.0, -1.0];
const WEST: P = [-1.0, 0.0, 0.0];

/// The lines of a gable roof 10 m along x over y in `[0, 8]`, eaves at
/// 3 m, ridge at 7 m, with a gable dormer at x in `[4, 6]` rising from the
/// south eaves (as the CLI test's body), and a few more: a hip, a
/// mono-pitch roof's top, a gable's foot (no roof feature).
fn gable(slack: f64) -> Vec<FaceEdge> {
    let dormer_west: P = [-1.0, 0.0, 1.0];
    vec![
        // Ridge.
        line(
            [0.0, 4.0, 7.0],
            [10.0, 4.0, 7.0],
            &[(SOUTH, [5.0, 2.0, 5.0]), (NORTH, [5.0, 6.0, 5.0])],
            slack,
        ),
        // Eaves, over the bottom.
        line(
            [0.0, 0.0, 3.0],
            [4.0, 0.0, 3.0],
            &[(DOWN, [2.0, 4.0, 3.0]), (SOUTH, [2.0, 1.0, 4.0])],
            slack,
        ),
        // Verge, beside the west gable.
        line(
            [0.0, 0.0, 3.0],
            [0.0, 4.0, 7.0],
            &[(SOUTH, [2.0, 2.0, 5.0]), (WEST, [0.0, 4.0, 4.0])],
            slack,
        ),
        // Valley between the dormer's west slope and the main roof.
        line(
            [4.0, 2.0, 5.0],
            [5.0, 3.0, 6.0],
            &[(SOUTH, [1.0, 4.0, 7.0]), (dormer_west, [4.0, 0.0, 5.0])],
            slack,
        ),
        // Abutment: the dormer's west cheek rising from the main roof.
        line(
            [4.0, 0.0, 3.0],
            [4.0, 2.0, 5.0],
            &[(SOUTH, [2.0, 1.0, 4.0]), (WEST, [4.0, 0.0, 5.0])],
            slack,
        ),
        // Hip of a hipped corner: south slope z = 3 + y, west slope
        // z = 3 + x.
        line(
            [0.0, 0.0, 3.0],
            [4.0, 4.0, 7.0],
            &[
                (SOUTH, [4.0, 1.0, 4.0]),
                ([-1.0, 0.0, 1.0], [1.0, 4.0, 4.0]),
            ],
            slack,
        ),
        // A mono-pitch roof's top against its back wall: a ridge.
        line(
            [0.0, 4.0, 7.0],
            [10.0, 4.0, 7.0],
            &[(SOUTH, [5.0, 2.0, 5.0]), ([0.0, 1.0, 0.0], [5.0, 4.0, 3.0])],
            slack,
        ),
        // The west gable's foot over the bottom: no roof feature.
        line(
            [0.0, 0.0, 3.0],
            [0.0, 8.0, 3.0],
            &[(DOWN, [5.0, 4.0, 3.0]), (WEST, [0.0, 4.0, 7.0])],
            slack,
        ),
    ]
}

fn listed(roofs: Roofs, model: Model, list: &str) -> Vec<MeasuredMember> {
    let (project, mut services) = model.services();
    services
        .register(VerticalExtentServiceHandle::new(Arc::new(roofs)))
        .unwrap();
    register_builtins(CapabilityRegistry::new())
        .unwrap()
        .install_measured(&mut services, &project);
    measured_members(&services, &id("roof"), list).unwrap()
}

fn text(member: &MeasuredMember, field: &str) -> Option<String> {
    match &member.fields[field] {
        MemberValue::Text { text } => Some(text.clone()),
        MemberValue::Undecided { .. } => None,
        other => panic!("{field} is {other:?}"),
    }
}

fn length(member: &MeasuredMember, field: &str) -> (f64, f64) {
    match &member.fields[field] {
        MemberValue::Measured(
            Measurement::Rounded { lower, upper, .. } | Measurement::Cited { lower, upper, .. },
        ) => (*lower, *upper),
        other => panic!("{field} is {other:?}"),
    }
}

fn near((lower, upper): (f64, f64), expected: f64) -> bool {
    lower <= expected && expected <= upper && upper - lower < 1e-9
}

fn roof(edges: Vec<FaceEdge>, exact: bool) -> Roofs {
    Roofs(BTreeMap::from([(id("roof"), (edges, exact))]))
}

/// #329: each line of a gable roof with a dormer is classified, and has
/// its length and heights; a gable's foot is no roof feature.
#[test]
fn a_gable_roof_with_a_dormer_yields_its_feature_lines() {
    let model = Model::default().object("roof", "roof");
    let members = listed(roof(gable(0.0), true), model, "roof_features");
    let features: Vec<Option<String>> = members.iter().map(|m| text(m, "feature")).collect();
    let expected = [
        "ridge", "eaves", "verge", "valley", "abutment", "hip", "ridge",
    ]
    .map(|name| Some(name.to_owned()));
    assert_eq!(features, expected);
    assert!(members.iter().all(|member| member.certain && member.exact));
    // The ridge: 10 m long at 7 m.
    assert!(near(length(&members[0], "length"), 10.0));
    assert!(near(length(&members[0], "low"), 7.0));
    assert!(near(length(&members[0], "high"), 7.0));
    // The eaves at 3 m; the verge from 3 m to 7 m; the valley from 5 m
    // to 6 m, √3 long.
    assert!(near(length(&members[1], "low"), 3.0));
    assert!(near(length(&members[1], "high"), 3.0));
    assert!(near(length(&members[2], "low"), 3.0));
    assert!(near(length(&members[2], "high"), 7.0));
    assert!(near(length(&members[2], "length"), 32.0_f64.sqrt()));
    assert!(near(length(&members[3], "low"), 5.0));
    assert!(near(length(&members[3], "high"), 6.0));
    let (lower, upper) = length(&members[3], "length");
    assert!(lower <= 3.0_f64.sqrt() && 3.0_f64.sqrt() <= upper);
    assert_eq!(
        text(&members[0], "what").unwrap(),
        "the ridge (0, 4, 7) - (10, 4, 7)"
    );
    // One class at a time.
    let ridges = listed(
        roof(gable(0.0), true),
        Model::default().object("roof", "roof"),
        "roof_features;feature=ridge",
    );
    assert_eq!(ridges.len(), 2);
    assert!(ridges.iter().all(|member| member.certain));
    let valleys = listed(
        roof(gable(0.0), true),
        Model::default().object("roof", "roof"),
        "roof_features;feature=valley",
    );
    assert_eq!(valleys.len(), 1);
}

/// Two faces meeting within the tolerance of flat, a flat roof's edge and
/// a line whose convexity the boxes leave open are undecided: possible
/// members of every class they might have, never guessed.
#[test]
fn an_ambiguous_line_is_undecided_never_guessed() {
    // A kink of about 0.7°: 45° below it, about 45.7° above.
    let kink = line(
        [0.0, 2.0, 5.0],
        [10.0, 2.0, 5.0],
        &[
            (SOUTH, [5.0, 1.0, 4.0]),
            ([0.0, -1.025, 1.0], [5.0, 3.0, 6.025]),
        ],
        0.0,
    );
    // A flat roof's edge over its fascia.
    let flat = line(
        [0.0, 0.0, 3.0],
        [10.0, 0.0, 3.0],
        &[
            ([0.0, 0.0, 1.0], [5.0, 5.0, 3.0]),
            ([0.0, -1.0, 0.0], [5.0, 0.0, 2.0]),
        ],
        0.0,
    );
    let members = listed(
        roof(vec![kink.clone(), flat.clone()], true),
        Model::default().object("roof", "roof"),
        "roof_features",
    );
    assert_eq!(members.len(), 2);
    for member in &members {
        assert!(!member.certain);
        assert!(text(member, "feature").is_none());
    }
    let why = |member: &MeasuredMember| match &member.fields["feature"] {
        MemberValue::Undecided { why } => why.clone(),
        other => panic!("{other:?}"),
    };
    assert!(
        why(&members[0]).contains("within the tolerance of flat"),
        "{}",
        why(&members[0])
    );
    assert!(
        why(&members[1]).contains("within the tolerance of level"),
        "{}",
        why(&members[1])
    );
    // A possible ridge: listed among the ridges, not surely one.
    let ridges = listed(
        roof(vec![kink.clone()], true),
        Model::default().object("roof", "roof"),
        "roof_features;feature=ridge",
    );
    assert_eq!(ridges.len(), 1);
    assert!(!ridges[0].certain);
    // Past a tolerance of half a degree the kink is a feature: the roof
    // steepens above it, so the faces meet concave, a valley.
    let members = listed(
        roof(vec![kink], true),
        Model::default().object("roof", "roof"),
        "roof_features;tolerance=0.5",
    );
    assert_eq!(text(&members[0], "feature").as_deref(), Some("valley"));
    // Normals known only within wide boxes leave the class open.
    let members = listed(
        roof(gable(0.6), false),
        Model::default().object("roof", "roof"),
        "roof_features",
    );
    assert!(members.iter().any(|member| !member.certain));
}

/// #329 on a tessellation: the lines are measured, never exact, and the
/// heights hold the true ones.
#[test]
fn a_feature_measured_on_a_tessellation_is_inexact() {
    let members = listed(
        roof(gable(0.001), false),
        Model::default().object("roof", "roof"),
        "roof_features;feature=ridge",
    );
    let ridge = &members[0];
    assert!(!ridge.exact);
    assert!(ridge.certain);
    assert!(matches!(
        ridge.fields["high"],
        MemberValue::Measured(Measurement::Cited { exact: false, .. })
    ));
    let (lower, upper) = length(ridge, "high");
    assert!(
        lower <= 7.0 - 0.001 && upper >= 7.0 + 0.001,
        "{lower} {upper}"
    );
}

/// Level placements, each stated exactly.
struct Levels(Vec<SourceSnapshot>, BTreeMap<ObjectId, f64>);

impl ObjectFrameService for Levels {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.0
    }

    fn object_frame(&self, object: &ObjectId) -> Result<ObjectFrame, ObjectFrameError> {
        let height = *self
            .1
            .get(object)
            .ok_or_else(|| ObjectFrameError::NotPlaced(object.clone()))?;
        let axis = |vector| MetricDirection::try_new(vector).unwrap();
        let frame = MetricFrame::try_new(
            MetricPoint::try_new(object.clone(), [0.0, 0.0, height]).unwrap(),
            axis([1.0, 0.0, 0.0]),
            axis([0.0, 1.0, 0.0]),
            axis([0.0, 0.0, 1.0]),
        )
        .unwrap();
        let evidence = Evidence::exact(source(), format!("placement:{}", object.local_id));
        ObjectFrame::try_new(object.clone(), frame, ObjectFront::NotStated, evidence)
    }
}

/// Heights above the level a path reaches: the ridge 4.5 m above a level
/// at 2.5 m.
#[test]
fn heights_are_measured_above_the_level_a_path_reaches() {
    let model = Model::default()
        .object("roof", "roof")
        .object("level", "storey")
        .edge("Storey", "roof", "level");
    let (project, mut services) = model.services();
    services
        .register(VerticalExtentServiceHandle::new(Arc::new(roof(
            gable(0.0),
            true,
        ))))
        .unwrap();
    services
        .register(ObjectFrameServiceHandle::new(Arc::new(Levels(
            vec![SourceSnapshot::try_new(source(), "r1", "sha256:1").unwrap()],
            BTreeMap::from([(id("level"), 2.5)]),
        ))))
        .unwrap();
    register_builtins(CapabilityRegistry::new())
        .unwrap()
        .install_measured(&mut services, &project);
    let members = measured_members(
        &services,
        &id("roof"),
        "roof_features;feature=ridge;level_path=Storey",
    )
    .unwrap();
    assert!(near(length(&members[0], "high"), 4.5));
    assert!(
        members[0]
            .evidence
            .iter()
            .any(|evidence| evidence.locator == "placement:level")
    );
}

fn member(name: &str) -> Value {
    json!({"kind": "property", "propertySet": "axioval:member", "property": name})
}

fn m(value: f64) -> Value {
    json!({"kind": "literal", "value": {"type": "quantity", "value": value, "unit": "m"}})
}

fn compare(operator: &str, left: Value, right: Value) -> Value {
    json!({"kind": "compare", "operator": operator, "left": left, "right": right})
}

fn requirement(id: &str, requirement: Value) -> Value {
    rule(
        id,
        EXPRESSION,
        "error",
        entity("roof"),
        json!({"requirement": {"type": "expression", "value": requirement}}),
        json!({}),
    )
}

fn checked(rules: Vec<Value>, roofs: Roofs) -> Report {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let definitions = definitions(&registry, &[EXPRESSION], &["roof"], &[], &[]);
    let plan = plan(&registry, &definitions, rules).unwrap();
    let session: EvidenceSession = session(Model::default().object("roof", "roof"))
        .with_host_service(
            VerticalExtentServiceHandle::new(Arc::new(roofs)),
            &[snapshot()],
        )
        .unwrap();
    run(registry, plan, &session, |runtime| runtime).unwrap()
}

/// A ridge height limit in a rule: 7 m passes 7.5 m and fails 6.5 m; with
/// a line of undecided class the limit is not evaluated, never passed.
#[test]
fn a_ridge_height_limit_is_judged_and_never_passed_on_an_undecided_line() {
    let highest = json!({"kind": "aggregate", "function": "max",
        "over": {"kind": "measured", "name": "roof_features;feature=ridge"},
        "value": member("high")});
    let report = checked(
        vec![
            requirement(
                "loose",
                compare("lessThanOrEquals", highest.clone(), m(7.5)),
            ),
            requirement(
                "tight",
                compare("lessThanOrEquals", highest.clone(), m(6.5)),
            ),
        ],
        roof(gable(0.0), true),
    );
    assert!(
        report.not_evaluated.is_empty(),
        "{:?}",
        report.not_evaluated
    );
    let findings = report.findings();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule_id.to_string(), "tight");
    // A kink of undecided class above the limit may be a ridge.
    let kink = line(
        [0.0, 2.0, 8.0],
        [10.0, 2.0, 8.0],
        &[
            (SOUTH, [5.0, 1.0, 7.0]),
            ([0.0, -1.025, 1.0], [5.0, 3.0, 9.025]),
        ],
        0.0,
    );
    let mut edges = gable(0.0);
    edges.push(kink);
    let report = checked(
        vec![requirement(
            "loose",
            compare("lessThanOrEquals", highest, m(7.5)),
        )],
        roof(edges, true),
    );
    assert!(report.findings().is_empty());
    assert_eq!(report.not_evaluated.len(), 1, "{:?}", report.not_evaluated);
    assert_eq!(
        report.not_evaluated[0].reason,
        NotEvaluatedReason::IncompleteEvidence
    );
}

/// Objects as points, each known within a slack (zero: exactly), measured
/// to a line between its ends' middles.
#[derive(Default)]
struct Points(BTreeMap<ObjectId, (P, f64)>);

impl axioval_engine::ProximityService for Points {
    fn bounds(
        &self,
        _: &ObjectId,
    ) -> Result<axioval_engine::ObjectBounds, axioval_engine::ProximityError> {
        Err(axioval_engine::ProximityError::Unavailable)
    }

    fn measure_proximity(
        &self,
        _: &axioval_engine::ProximityRequest,
    ) -> Result<axioval_engine::ProximityEvidence, axioval_engine::ProximityError> {
        Err(axioval_engine::ProximityError::Unavailable)
    }

    fn measure_line_distance(
        &self,
        request: &axioval_engine::LineDistanceRequest,
    ) -> Result<axioval_engine::LineDistanceEvidence, axioval_engine::ProximityError> {
        let (point, slack) = *self
            .0
            .get(request.object())
            .ok_or(axioval_engine::ProximityError::Unavailable)?;
        let middle =
            |at: PointBox| [0, 1, 2].map(|axis| f64::midpoint(at.lower()[axis], at.upper()[axis]));
        let (a, b) = (middle(request.start()), middle(request.end()));
        let along = sub(b, a);
        let t = (dot(sub(point, a), along) / dot(along, along)).clamp(0.0, 1.0);
        let foot = [0, 1, 2].map(|axis| a[axis] + t * along[axis]);
        let gap = sub(point, foot);
        let distance = dot(gap, gap).sqrt();
        let mut evidence = Evidence::exact(source(), format!("line:{}", request.object().local_id));
        evidence.exact = slack == 0.0 && request.start().is_exact();
        axioval_engine::LineDistanceEvidence::try_new(
            request.clone(),
            (distance - slack).max(0.0),
            distance + slack,
            evidence,
        )
    }
}

fn distance_services(
    roofs: Roofs,
    points: Points,
) -> (axioval_ir::Project, axioval_engine::ServiceRegistry) {
    let model = Model::default()
        .object("roof", "roof")
        .object("skylight", "window")
        .object("chimney", "window");
    let (project, mut services) = model.services();
    services
        .register(VerticalExtentServiceHandle::new(Arc::new(roofs)))
        .unwrap();
    services
        .register(axioval_engine::ProximityServiceHandle::new(Arc::new(
            points,
        )))
        .unwrap();
    register_builtins(CapabilityRegistry::new())
        .unwrap()
        .install_measured(&mut services, &project);
    (project, services)
}

/// #330: a skylight 0.4 m from the valley measures 0.4 m, exactly, and
/// relates the roof; the nearest of two ridges is taken; a roof without a
/// hip has no distance to one.
#[test]
fn a_skylight_measures_its_distance_to_the_nearest_line_of_a_class() {
    // 0.4 m off the valley's middle (4.5, 2.5, 5.5), square to it and to
    // the main roof's normal.
    let off = 0.4 / 6.0_f64.sqrt();
    let skylight = [4.5 - 2.0 * off, 2.5 + off, 5.5 + off];
    let points = Points(BTreeMap::from([
        (id("skylight"), (skylight, 0.0)),
        (id("chimney"), ([5.0, 4.0, 9.0], 0.0)),
    ]));
    let (project, services) = distance_services(roof(gable(0.0), true), points);
    let measured =
        |object: &str, name: &str| common::measured_cited(&services, &project, &id(object), name);
    let ((lower, upper), exact) =
        measured("skylight", "distance_to_feature;feature=valley;roofs=roof")
            .unwrap()
            .unwrap();
    assert!(lower <= 0.4 + 1e-12 && upper >= 0.4 - 1e-12 && upper - lower < 1e-9);
    assert!(exact);
    // The chimney 2 m above the ridge at (5, 4, 7).
    let ((lower, upper), _) = measured("chimney", "distance_to_feature;feature=ridge;roofs=roof")
        .unwrap()
        .unwrap();
    assert!(
        lower <= 2.0 + 1e-12 && upper >= 2.0 - 1e-12,
        "{lower} {upper}"
    );
    // Without a line of the class: none.
    let (project, services) = distance_services(
        roof(gable(0.0)[..5].to_vec(), true),
        Points(BTreeMap::from([(id("skylight"), (skylight, 0.0))])),
    );
    assert_eq!(
        common::measured_cited(
            &services,
            &project,
            &id("skylight"),
            "distance_to_feature;feature=hip;roofs=roof"
        )
        .unwrap(),
        None
    );
}

/// A line that may be of the class and lies nearer than the nearest that
/// surely is leaves the distance not evaluated; one farther away does not
/// matter.
#[test]
fn a_possible_line_nearer_than_the_nearest_sure_one_leaves_the_distance_open() {
    let kink = line(
        [0.0, 2.0, 5.0],
        [10.0, 2.0, 5.0],
        &[
            (SOUTH, [5.0, 1.0, 4.0]),
            ([0.0, -1.025, 1.0], [5.0, 3.0, 6.025]),
        ],
        0.0,
    );
    let mut edges = gable(0.0);
    edges.push(kink);
    let points = Points(BTreeMap::from([
        (id("skylight"), ([5.0, 2.0, 6.0], 0.0)),
        (id("chimney"), ([5.0, 4.0, 9.0], 0.0)),
    ]));
    let (project, services) = distance_services(roof(edges, true), points);
    let why = common::measured_cited(
        &services,
        &project,
        &id("skylight"),
        "distance_to_feature;feature=ridge;roofs=roof",
    )
    .unwrap_err();
    assert!(why.contains("may be a ridge nearer"), "{why}");
    // The chimney is nearer the ridge than the kink.
    let ((lower, _), _) = common::measured_cited(
        &services,
        &project,
        &id("chimney"),
        "distance_to_feature;feature=ridge;roofs=roof",
    )
    .unwrap()
    .unwrap();
    assert!(lower <= 2.0 + 1e-12);
}

/// #330 on a tessellation: the distance is measured, never exact.
#[test]
fn a_distance_to_a_feature_on_a_tessellation_is_inexact() {
    let points = Points(BTreeMap::from([(id("skylight"), ([5.0, 4.0, 9.0], 0.01))]));
    let (project, services) = distance_services(roof(gable(0.001), false), points);
    let ((lower, upper), exact) = common::measured_cited(
        &services,
        &project,
        &id("skylight"),
        "distance_to_feature;feature=ridge;roofs=roof",
    )
    .unwrap()
    .unwrap();
    assert!(!exact);
    assert!(lower <= 1.99 && upper >= 2.01, "{lower} {upper}");
}
