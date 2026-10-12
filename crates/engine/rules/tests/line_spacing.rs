//! Repeated members along a line (`line_spacing`): posts of a fence placed
//! along it by their extents, ordered by station, with the gaps between
//! them and at the fence's ends, read by expression rules through `pairs`.
#![allow(missing_docs, clippy::needless_pass_by_value)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    AlignmentError, AlignmentInterval, AlignmentPosition, AlignmentRequest, AlignmentService,
    AlignmentServiceHandle, CapabilityRegistry, DirectionalExtent, ElevationInterval,
    EvidenceSession, MeasuredMember, Measurement, MemberValue, MetricDirection, VerticalExtent,
    VerticalExtentError, VerticalExtentService, VerticalExtentServiceHandle, measured_members,
};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, Report};
use axioval_rules::register_builtins;
use common::runtime::{definitions, entity, plan, rule, run, session, snapshot};
use common::{Model, id, source};
use serde_json::{Value, json};

const EXPRESSION: &str = "axioval:capability.expression";

/// Boxes from `low` to `high` in world coordinates, each end of an extent
/// known within `slack` (zero: exactly).
#[derive(Default)]
struct Boxes(BTreeMap<ObjectId, ([f64; 3], [f64; 3], f64)>);

impl Boxes {
    fn with(mut self, local: &str, low: [f64; 3], high: [f64; 3], slack: f64) -> Self {
        self.0.insert(id(local), (low, high, slack));
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
        let (low, high, slack) = *self
            .0
            .get(object)
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?;
        let d = along.components();
        let (mut least, mut most) = (0.0, 0.0);
        for axis in 0..3 {
            let (a, b) = (d[axis] * low[axis], d[axis] * high[axis]);
            least += a.min(b);
            most += a.max(b);
        }
        let mut evidence = Evidence::exact(source(), format!("extent:{}", object.local_id));
        evidence.exact = slack == 0.0;
        DirectionalExtent::try_new(
            object.clone(),
            along,
            ElevationInterval::try_new(least - slack, least + slack)?,
            ElevationInterval::try_new(most - slack, most + slack)?,
            evidence,
        )
    }
}

/// A fence `f1` 10 m long along x hosting posts 0.1 m wide centred at
/// `centres`, each end known within `slack`.
fn fence(centres: &[(&str, f64)], slack: f64) -> (Model, Boxes) {
    let mut model = Model::default().object("f1", "slab");
    let mut boxes = Boxes::default().with("f1", [0.0, 0.0, 0.0], [10.0, 0.1, 1.0], 0.0);
    for (post, centre) in centres {
        model = model.object(post, "pipe").edge("Hosts", "f1", post);
        boxes = boxes.with(
            post,
            [centre - 0.05, 0.0, 0.0],
            [centre + 0.05, 0.1, 1.2],
            slack,
        );
    }
    (model, boxes)
}

/// Five posts with gaps of 2.0, 2.0, 2.4 and 2.0 m between their centres,
/// 1.0 m from the fence's start and 0.6 m from its end; listed out of
/// order.
fn posts() -> Vec<(&'static str, f64)> {
    vec![
        ("p4", 7.4),
        ("p1", 1.0),
        ("p2", 3.0),
        ("p5", 9.4),
        ("p3", 5.0),
    ]
}

const LIST: &str = "line_spacing;members=pipe;member_path=Hosts;direction=1,0,0";

fn listed(model: Model, boxes: Boxes, list: &str) -> Vec<MeasuredMember> {
    let (project, mut services) = model.services();
    services
        .register(VerticalExtentServiceHandle::new(Arc::new(boxes)))
        .unwrap();
    register_builtins(CapabilityRegistry::new())
        .unwrap()
        .install_measured(&mut services, &project);
    measured_members(&services, &id("f1"), list).unwrap()
}

/// A length field: its interval, `None` where it is `null`.
fn length(member: &MeasuredMember, field: &str) -> Option<(f64, f64)> {
    match &member.fields[field] {
        MemberValue::Measured(
            Measurement::Rounded { lower, upper, .. }
            | Measurement::Cited { lower, upper, .. }
            | Measurement::Value { lower, upper, .. },
        ) => Some((*lower, *upper)),
        MemberValue::Measured(Measurement::Absent { .. }) => None,
        other => panic!("{field} is {other:?}"),
    }
}

fn near(found: Option<(f64, f64)>, expected: f64) -> bool {
    found.is_some_and(|(lower, upper)| {
        lower <= expected + 1e-9 && upper >= expected - 1e-9 && upper - lower < 1e-9
    })
}

fn member_of(member: &MeasuredMember) -> String {
    match &member.fields["member"] {
        MemberValue::Objects { objects } => objects[0].local_id.clone(),
        other => panic!("{other:?}"),
    }
}

/// #342: five posts along a line with gaps of 2.0, 2.0, 2.4 and 2.0 m
/// yield those gaps, in station order, and the gaps at the ends; clear
/// gaps are between the posts' faces.
#[test]
fn five_posts_yield_their_gaps_in_station_order() {
    let (model, boxes) = fence(&posts(), 0.0);
    let members = listed(model, boxes, LIST);
    let order: Vec<String> = members.iter().map(member_of).collect();
    assert_eq!(order, ["p1", "p2", "p3", "p4", "p5"]);
    let stations = [1.0, 3.0, 5.0, 7.4, 9.4];
    for (member, station) in members.iter().zip(stations) {
        assert!(near(length(member, "station"), station), "{member:?}");
        assert!(near(length(member, "from"), station - 0.05), "{member:?}");
        assert!(near(length(member, "to"), station + 0.05), "{member:?}");
        assert!(member.exact);
    }
    assert_eq!(length(&members[0], "gap"), None);
    for (member, gap) in members[1..].iter().zip([2.0, 2.0, 2.4, 2.0]) {
        assert!(near(length(member, "gap"), gap), "{member:?}");
    }
    assert!(near(length(&members[0], "start_gap"), 1.0));
    assert!(near(length(&members[4], "end_gap"), 0.6));
    assert!(
        members[1..]
            .iter()
            .all(|member| length(member, "start_gap").is_none())
    );
    assert!(
        members[..4]
            .iter()
            .all(|member| length(member, "end_gap").is_none())
    );
    let truth = |member: &MeasuredMember, field: &str| match &member.fields[field] {
        MemberValue::Truth { value, .. } => *value,
        other => panic!("{other:?}"),
    };
    assert!(truth(&members[0], "first") && !truth(&members[1], "first"));
    assert!(truth(&members[4], "last") && !truth(&members[3], "last"));
    // Clear gaps lie between the faces, 0.1 m less, and the end gaps to the
    // faces nearest the ends.
    let (model, boxes) = fence(&posts(), 0.0);
    let clear = listed(model, boxes, &format!("{LIST};gap=clear"));
    for (member, gap) in clear[1..].iter().zip([1.9, 1.9, 2.3, 1.9]) {
        assert!(near(length(member, "gap"), gap), "{member:?}");
    }
    assert!(near(length(&clear[0], "start_gap"), 0.95));
    assert!(near(length(&clear[4], "end_gap"), 0.55));
}

/// Stations known only within intervals keep their gaps as intervals and
/// never exact; posts whose stations may come either way round leave the
/// gaps around them undecided, never ordered by a guess.
#[test]
fn a_spacing_measured_on_a_tessellation_is_inexact() {
    let (model, boxes) = fence(&posts(), 0.01);
    let members = listed(model, boxes, LIST);
    for member in &members {
        assert!(!member.exact);
        assert!(matches!(
            member.fields["station"],
            MemberValue::Measured(Measurement::Cited { exact: false, .. })
        ));
    }
    let (lower, upper) = length(&members[3], "gap").unwrap();
    assert!(
        lower <= 2.4 - 0.02 + 1e-9 && upper >= 2.4 + 0.02 - 1e-9,
        "{lower} {upper}"
    );
    // Two posts 0.01 m apart, each end known within 0.05 m: which comes
    // first is undecided.
    let (model, boxes) = fence(&[("p1", 1.0), ("p2", 3.0), ("p3", 3.01)], 0.05);
    let members = listed(model, boxes, LIST);
    let (lower, upper) = length(&members[0], "start_gap").unwrap();
    assert!(
        lower <= 0.95 + 1e-9 && upper >= 1.05 - 1e-9,
        "{lower} {upper}"
    );
    for (place, field) in [
        (1, "gap"),
        (2, "gap"),
        (2, "last"),
        (1, "last"),
        (2, "end_gap"),
    ] {
        assert!(
            matches!(members[place].fields[field], MemberValue::Undecided { .. }),
            "{place} {field}: {:?}",
            members[place].fields[field]
        );
    }
    assert!(matches!(
        members[0].fields["first"],
        MemberValue::Truth { value: true, .. }
    ));
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

/// `function` over consecutive posts of the fence, by station.
fn pairs(function: &str, value: Value) -> Value {
    json!({"kind": "pairs", "function": function, "over": {"kind": "measured", "name": LIST},
        "by": member("station"), "value": value})
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

fn checked(rules: Vec<Value>, model: Model, boxes: Boxes) -> Report {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let definitions = definitions(&registry, &[EXPRESSION], &["slab", "pipe"], &[], &[]);
    let plan = plan(&registry, &definitions, rules).unwrap();
    let session: EvidenceSession = session(model)
        .with_host_service(
            VerticalExtentServiceHandle::new(Arc::new(boxes)),
            &[snapshot()],
        )
        .unwrap();
    run(registry, plan, &session, |runtime| runtime).unwrap()
}

/// #342 in a rule: the largest gap is 2.4 m; a limit of 2.2 m between
/// consecutive posts is missed at p3 → p4, which the finding names and
/// relates; the gap at the start is read of the first post.
#[test]
fn the_largest_gap_and_the_gap_missing_a_limit_are_found() {
    let (model, boxes) = fence(&posts(), 0.0);
    let report = checked(
        vec![
            requirement(
                "largest",
                json!({"kind": "between", "operand": pairs("max", member("gap")),
                    "low": m(2.399), "high": m(2.401)}),
            ),
            requirement(
                "limit",
                pairs("all", compare("lessThanOrEquals", member("gap"), m(2.2))),
            ),
            requirement(
                "start",
                compare(
                    "lessThanOrEquals",
                    json!({"kind": "argmin", "over": {"kind": "measured", "name": LIST},
                        "by": member("station"), "value": member("start_gap")}),
                    m(0.8),
                ),
            ),
        ],
        model,
        boxes,
    );
    assert!(
        report.not_evaluated.is_empty(),
        "{:?}",
        report.not_evaluated
    );
    let findings = |rule: &str| -> Vec<&axioval_ir::Finding> {
        report
            .findings()
            .iter()
            .filter(|finding| finding.rule_id.to_string() == rule)
            .collect()
    };
    assert!(findings("largest").is_empty());
    let limit = findings("limit");
    assert_eq!(limit.len(), 1);
    assert_eq!(
        limit[0].related,
        [id("p3"), id("p4")],
        "{}",
        limit[0].message
    );
    let pair = format!("pairs {} → {}", id("p3"), id("p4"));
    assert!(limit[0].message.contains(&pair), "{}", limit[0].message);
    // The first post stands 1.0 m from the start.
    assert_eq!(findings("start").len(), 1);
    // Posts whose order is undecided leave the limit open, never passed.
    let (model, boxes) = fence(&[("p1", 1.0), ("p2", 3.0), ("p3", 3.01)], 0.05);
    let report = checked(
        vec![requirement(
            "limit",
            pairs("all", compare("lessThanOrEquals", member("gap"), m(2.2))),
        )],
        model,
        boxes,
    );
    assert!(report.findings().is_empty());
    assert_eq!(report.not_evaluated.len(), 1, "{:?}", report.not_evaluated);
    assert_eq!(
        report.not_evaluated[0].reason,
        NotEvaluatedReason::IncompleteEvidence
    );
}

/// Stations along an alignment, by object, each exact.
struct Stations(BTreeMap<ObjectId, f64>);

impl AlignmentService for Stations {
    fn measure_alignment_position(
        &self,
        request: &AlignmentRequest,
    ) -> Result<AlignmentPosition, AlignmentError> {
        let station = *self
            .0
            .get(request.object())
            .ok_or_else(|| AlignmentError::Unavailable("no station".into()))?;
        let at = |value: f64| AlignmentInterval::try_new(value, value);
        AlignmentPosition::try_new(
            request.clone(),
            at(station)?,
            at(station)?,
            at(1.5)?,
            at(0.0)?,
            Evidence::exact(source(), format!("station:{}", request.object().local_id)),
        )
    }
}

/// Along a curved alignment the gaps are differences of the `station`
/// measured value, related by `pairs`: bollards at stations 10, 12, 14,
/// 16.4 and 18.4 m have a largest gap of 2.4 m and miss a 2.2 m limit
/// between the third and the fourth.
#[test]
fn gaps_along_an_alignment_are_station_differences_by_pairs() {
    let mut model = Model::default()
        .object("a1", "alignment")
        .object("r1", "slab");
    let mut stations = BTreeMap::new();
    for (bollard, station) in [
        ("b4", 16.4),
        ("b1", 10.0),
        ("b2", 12.0),
        ("b5", 18.4),
        ("b3", 14.0),
    ] {
        model = model.object(bollard, "pipe").edge("Hosts", "r1", bollard);
        stations.insert(id(bollard), station);
    }
    let station = json!({"kind": "property", "propertySet": "axioval:measured",
        "property": "station;alignment=alignment"});
    let gap = json!({"kind": "subtract", "left": station,
        "right": {"kind": "previous", "operand": station}});
    let over = json!({"kind": "path", "path": ["Hosts"]});
    let limit = json!({"kind": "pairs", "function": "all", "over": over, "by": station,
        "value": compare("lessThanOrEquals", gap.clone(), m(2.2))});
    let largest = json!({"kind": "between",
        "operand": {"kind": "pairs", "function": "max", "over": over, "by": station,
            "value": gap},
        "low": m(2.399), "high": m(2.401)});
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let definitions = definitions(
        &registry,
        &[EXPRESSION],
        &["slab", "pipe", "alignment"],
        &[],
        &[],
    );
    let plan = plan(
        &registry,
        &definitions,
        vec![requirement("limit", limit), requirement("largest", largest)],
    )
    .unwrap();
    let session = session(model)
        .with_host_service(
            AlignmentServiceHandle::new(Arc::new(Stations(stations))),
            &[snapshot()],
        )
        .unwrap();
    let report = run(registry, plan, &session, |runtime| runtime).unwrap();
    assert!(
        report.not_evaluated.is_empty(),
        "{:?}",
        report.not_evaluated
    );
    let findings = report.findings();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule_id.to_string(), "limit");
    assert_eq!(
        findings[0].related,
        [id("b3"), id("b4")],
        "{}",
        findings[0].message
    );
}
