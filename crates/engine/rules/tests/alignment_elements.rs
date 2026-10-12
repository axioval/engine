//! The elements of an alignment's layouts (`alignment_elements`): each
//! element's kind, stations, radii and a clothoid's parameter, read by
//! expression rules relating consecutive elements through `pairs`.
#![allow(missing_docs, clippy::needless_pass_by_value)]

mod common;

use std::sync::Arc;

use axioval_engine::{
    AlignmentElement, AlignmentElementKind, AlignmentElements, AlignmentElementsRequest,
    AlignmentError, AlignmentInterval, AlignmentLayout, AlignmentPosition, AlignmentRequest,
    AlignmentService, AlignmentServiceHandle, CapabilityRegistry, ElementEnd, MeasuredMember,
    Measurement, MemberValue, measured_members,
};
use axioval_ir::{Evidence, Report};
use axioval_rules::register_builtins;
use common::runtime::{definitions, entity, plan, rule, run, session, snapshot};
use common::{Model, id, source};
use serde_json::{Value, json};

const EXPRESSION: &str = "axioval:capability.expression";
const PLAN: &str = "alignment_elements;layout=horizontal";
const PROFILE: &str = "alignment_elements;layout=vertical";

/// The layouts of `a1`: a line, a clothoid into a radius of 500 m, that
/// arc, a clothoid from it into 1000 m and that arc; in the vertical a
/// constant gradient and a vertical clothoid stating no curvature.
/// Stations are labelled within `slack` (zero: exactly).
struct Layouts {
    slack: f64,
}

impl AlignmentService for Layouts {
    fn measure_alignment_position(
        &self,
        request: &AlignmentRequest,
    ) -> Result<AlignmentPosition, AlignmentError> {
        Err(AlignmentError::UnknownObject(request.object().clone()))
    }

    fn measure_alignment_elements(
        &self,
        request: &AlignmentElementsRequest,
    ) -> Result<AlignmentElements, AlignmentError> {
        if request.alignment() != &id("a1") {
            return Err(AlignmentError::NotAlignment(request.alignment().clone()));
        }
        let at = |station: f64| {
            AlignmentInterval::try_new(station - self.slack, station + self.slack).unwrap()
        };
        let radius = ElementEnd::Radius;
        let element = |kind, name: &str, (from, to): (f64, f64), ends, gradients| {
            AlignmentElement::try_new(
                kind,
                name,
                format!("#{from} ({name})"),
                (at(from), at(to)),
                to - from,
                ends,
                gradients,
            )
            .unwrap()
        };
        let elements = match request.layout() {
            AlignmentLayout::Horizontal => vec![
                element(
                    AlignmentElementKind::Straight,
                    "LINE",
                    (0.0, 100.0),
                    (ElementEnd::Straight, ElementEnd::Straight),
                    None,
                ),
                element(
                    AlignmentElementKind::Clothoid,
                    "CLOTHOID",
                    (100.0, 200.0),
                    (ElementEnd::Straight, radius(500.0)),
                    None,
                ),
                element(
                    AlignmentElementKind::Arc,
                    "CIRCULARARC",
                    (200.0, 300.0),
                    (radius(500.0), radius(500.0)),
                    None,
                ),
                element(
                    AlignmentElementKind::Clothoid,
                    "CLOTHOID",
                    (300.0, 380.0),
                    (radius(500.0), radius(1000.0)),
                    None,
                ),
                element(
                    AlignmentElementKind::Arc,
                    "CIRCULARARC",
                    (380.0, 480.0),
                    (radius(1000.0), radius(1000.0)),
                    None,
                ),
            ],
            AlignmentLayout::Vertical => vec![
                element(
                    AlignmentElementKind::Straight,
                    "CONSTANTGRADIENT",
                    (0.0, 200.0),
                    (ElementEnd::Straight, ElementEnd::Straight),
                    Some((0.01, 0.01)),
                ),
                element(
                    AlignmentElementKind::Clothoid,
                    "CLOTHOID",
                    (200.0, 480.0),
                    (
                        ElementEnd::Unread("a vertical clothoid states no curvature".into()),
                        ElementEnd::Unread("a vertical clothoid states no curvature".into()),
                    ),
                    Some((0.01, 0.03)),
                ),
            ],
        };
        let mut evidence = Evidence::exact(source(), "layouts of a1");
        evidence.exact = self.slack == 0.0;
        AlignmentElements::try_new(request.clone(), elements, evidence)
    }
}

fn model() -> Model {
    Model::default().object("a1", "alignment")
}

fn listed(slack: f64, list: &str) -> Vec<MeasuredMember> {
    let (project, mut services) = model().services();
    services
        .register(AlignmentServiceHandle::new(Arc::new(Layouts { slack })))
        .unwrap();
    register_builtins(CapabilityRegistry::new())
        .unwrap()
        .install_measured(&mut services, &project);
    measured_members(&services, &id("a1"), list).unwrap()
}

/// A number field: its interval, `None` where it is `null`.
fn number(member: &MeasuredMember, field: &str) -> Option<(f64, f64)> {
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

fn holds(found: Option<(f64, f64)>, expected: f64) {
    let (lower, upper) = found.expect("a value");
    assert!(
        lower <= expected && upper >= expected && upper - lower < 1e-9,
        "[{lower}, {upper}] does not hold {expected}"
    );
}

fn text(member: &MeasuredMember, field: &str) -> String {
    match &member.fields[field] {
        MemberValue::Text { text } => text.clone(),
        other => panic!("{field} is {other:?}"),
    }
}

fn undecided(member: &MeasuredMember, field: &str) -> String {
    match &member.fields[field] {
        MemberValue::Undecided { why } => why.clone(),
        other => panic!("{field} is {other:?}"),
    }
}

/// #337: the plan's elements in order, each clothoid with its parameter
/// (`A^2 = L / |k_end - k_start|`), each arc with its radius; a straight's
/// radius is unbounded, so undecided, never a number.
#[test]
fn the_elements_yield_their_radii_and_clothoid_parameters() {
    let members = listed(0.0, PLAN);
    assert_eq!(members.len(), 5);
    let kinds: Vec<_> = members.iter().map(|member| text(member, "kind")).collect();
    assert_eq!(kinds, ["straight", "clothoid", "arc", "clothoid", "arc"]);
    for (place, member) in members.iter().enumerate() {
        assert!(member.exact);
        #[allow(clippy::cast_precision_loss)]
        let index = (place + 1) as f64;
        holds(number(member, "index"), index);
    }
    let [line, into, arc, between, wide] = members.as_slice() else {
        unreachable!()
    };
    // The line: straight, zero curvature, no radius, no parameter.
    holds(number(line, "start_curvature"), 0.0);
    assert!(undecided(line, "start_radius").contains("unbounded"));
    assert!(undecided(line, "radius").contains("unbounded"));
    assert_eq!(number(line, "clothoid_parameter"), None);
    assert_eq!(number(line, "start_gradient"), None);
    // A clothoid from straight into 500 m over 100 m: A^2 = 50000.
    holds(number(into, "clothoid_parameter"), 50_000.0_f64.sqrt());
    assert!(undecided(into, "start_radius").contains("unbounded"));
    holds(number(into, "end_radius"), 500.0);
    holds(number(into, "end_curvature"), 1.0 / 500.0);
    assert_eq!(number(into, "radius"), None);
    assert_eq!(text(into, "type"), "CLOTHOID");
    holds(number(arc, "radius"), 500.0);
    holds(number(arc, "from"), 200.0);
    holds(number(arc, "length"), 100.0);
    // From 500 m into 1000 m over 80 m: A^2 = 80 / (1/500 - 1/1000).
    holds(number(between, "clothoid_parameter"), 80_000.0_f64.sqrt());
    holds(number(wide, "radius"), 1000.0);

    // In the vertical: gradients stated, a vertical clothoid's curvature
    // unread, so its radius and parameter undecided.
    let members = listed(0.0, PROFILE);
    let [grade, clothoid] = members.as_slice() else {
        panic!("{members:?}")
    };
    holds(number(grade, "start_gradient"), 0.01);
    holds(number(grade, "start_curvature"), 0.0);
    holds(number(clothoid, "end_gradient"), 0.03);
    for field in [
        "start_curvature",
        "end_radius",
        "radius",
        "clothoid_parameter",
    ] {
        assert!(
            undecided(clothoid, field).contains("states no curvature"),
            "{field}"
        );
    }
}

/// Stations the service labels only within an interval leave every
/// member's evidence inexact, the stations intervals.
#[test]
fn elements_labelled_with_rounded_stations_are_inexact() {
    let members = listed(0.01, PLAN);
    for member in &members {
        assert!(!member.exact);
    }
    let (lower, upper) = number(&members[2], "from").unwrap();
    assert!(lower <= 199.99 && upper >= 200.01, "{lower} {upper}");
}

fn member(name: &str) -> Value {
    json!({"kind": "property", "propertySet": "axioval:member", "property": name})
}

fn previous(name: &str) -> Value {
    json!({"kind": "previous", "operand": member(name)})
}

fn requirement(id: &str, requirement: Value) -> Value {
    rule(
        id,
        EXPRESSION,
        "error",
        entity("alignment"),
        json!({"requirement": {"type": "expression", "value": requirement}}),
        json!({}),
    )
}

fn checked(rules: Vec<Value>) -> Report {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let definitions = definitions(&registry, &[EXPRESSION], &["alignment"], &[], &[]);
    let plan = plan(&registry, &definitions, rules).unwrap();
    let session = session(model())
        .with_host_service(
            AlignmentServiceHandle::new(Arc::new(Layouts { slack: 0.0 })),
            &[snapshot()],
        )
        .unwrap();
    run(registry, plan, &session, |runtime| runtime).unwrap()
}

/// #337 in a rule: a clothoid's parameter is at least a third of the
/// radius of the arc it leads into. The first meets it (223.6 m against
/// 166.7 m), the second misses it (282.8 m against 333.3 m), and the
/// largest parameter is read by `pairs` over the consecutive elements.
#[test]
fn consecutive_elements_are_related_by_pairs() {
    let clothoid_before = json!({"kind": "compare", "operator": "equals",
        "left": previous("kind"),
        "right": {"kind": "literal", "value": {"type": "string", "value": "clothoid"}}});
    let third = json!({"kind": "divide", "left": member("radius"),
        "right": {"kind": "literal", "value": {"type": "number", "value": 3.0}}});
    let enough = json!({"kind": "compare", "operator": "greaterThanOrEquals",
        "left": previous("clothoid_parameter"), "right": third});
    let parameter = json!({"kind": "pairs", "function": "all",
        "over": {"kind": "measured", "name": PLAN}, "by": member("index"),
        "value": {"kind": "implies", "antecedent": clothoid_before, "consequent": enough}});
    let report = checked(vec![requirement("parameter", parameter)]);
    assert!(
        report.not_evaluated.is_empty(),
        "{:?}",
        report.not_evaluated
    );
    let findings = report.findings();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule_id.to_string(), "parameter");
}
