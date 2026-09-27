//! Distance capability: counting modes, projections and container scoping.
//!
//! The stub answers only the pairs and projections a test declares and panics
//! on any other, which also proves the projected broad phase kept irrelevant
//! pairs away from the narrow phase.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    Bounds3, CapabilityEvaluation, GeometryFidelity, NotEvaluatedReason, ObjectBounds,
    ProjectedDistanceEvidence, ProximityError, ProximityEvidence, ProximityRequest,
    ProximityService, ProximityServiceHandle, RuleCapability,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Evidence, ObjectId};
use axioval_rules::Distance;
use common::{Model, id, integer, kind, number, rule, selector, source, string, strings};

const CAPABILITY: &str = "axioval:capability.distance";

#[derive(Default)]
struct Stub {
    /// Unit box at `(x, z)` per object; `None` means no geometry.
    boxes: BTreeMap<String, Option<(f64, f64, GeometryFidelity)>>,
    /// `(subject, counterpart, projection)` -> distance interval.
    distances: BTreeMap<(String, String, String), (f64, f64)>,
}

fn tessellated() -> GeometryFidelity {
    GeometryFidelity::tessellated(0.05).unwrap()
}

impl Stub {
    fn at(mut self, local: &str, x: f64, z: f64) -> Self {
        self.boxes
            .insert(local.into(), Some((x, z, GeometryFidelity::Exact)));
        self
    }
    fn curved(mut self, local: &str, x: f64, z: f64) -> Self {
        self.boxes.insert(local.into(), Some((x, z, tessellated())));
        self
    }
    fn without_geometry(mut self, local: &str) -> Self {
        self.boxes.insert(local.into(), None);
        self
    }
    fn distance(mut self, a: &str, b: &str, projection: &str, interval: (f64, f64)) -> Self {
        self.distances
            .insert((a.into(), b.into(), projection.into()), interval);
        self
    }
    fn fidelity(&self, local: &str) -> GeometryFidelity {
        self.boxes[local].unwrap().2
    }
}

impl ProximityService for Stub {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        let (x, z, fidelity) = self
            .boxes
            .get(&object.local_id)
            .copied()
            .flatten()
            .ok_or(ProximityError::Unavailable)?;
        ObjectBounds::try_new(
            object.clone(),
            Bounds3::try_new([x, 0.0, z], [x + 1.0, 1.0, z + 1.0])?,
            fidelity,
        )
    }

    fn measure_proximity(&self, _: &ProximityRequest) -> Result<ProximityEvidence, ProximityError> {
        panic!("distance measures through measure_distance")
    }

    fn measure_distance(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProjectedDistanceEvidence, ProximityError> {
        let (a, b) = (
            request.subject().local_id.clone(),
            request.counterpart().local_id.clone(),
        );
        let projection = format!("{:?}", request.projection());
        let interval = *self
            .distances
            .get(&(a.clone(), b.clone(), projection.clone()))
            .or_else(|| {
                self.distances
                    .get(&(b.clone(), a.clone(), projection.clone()))
            })
            .unwrap_or_else(|| panic!("broad phase should have pruned {a}/{b} in {projection}"));
        let fidelity = self.fidelity(&a).combined(self.fidelity(&b));
        ProjectedDistanceEvidence::try_new(
            request.clone(),
            interval.0,
            interval.1,
            fidelity,
            Evidence {
                source: source(),
                locator: format!("distance:{a}:{b}"),
                exact: fidelity.is_exact(),
            },
        )
    }
}

/// A service that only measures in space, through the default method.
struct SpaceOnly;

impl ProximityService for SpaceOnly {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        ObjectBounds::try_new(
            object.clone(),
            Bounds3::try_new([0.0; 3], [1.0; 3])?,
            GeometryFidelity::Exact,
        )
    }
    fn measure_proximity(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProximityEvidence, ProximityError> {
        ProximityEvidence::try_new(
            request.clone(),
            0.5,
            Some(0.0),
            0.0,
            None,
            GeometryFidelity::Exact,
            Evidence::exact(source(), "space"),
        )
    }
}

fn model() -> Model {
    Model::default()
        .object("pipe", "pipe")
        .object("near", "wall")
        .object("mid", "wall")
        .object("far", "wall")
}

fn check(parameters: Vec<(&str, ParameterValue)>) -> axioval_engine::CompiledRule {
    let mut all = vec![("counterparts", selector(kind("wall")))];
    all.extend(parameters);
    rule(CAPABILITY, kind("pipe"), all)
}

fn run(
    model: Model,
    service: impl ProximityService,
    parameters: Vec<(&str, ParameterValue)>,
) -> CapabilityEvaluation {
    let service = Arc::new(service);
    model.evaluate_with(&Distance, &check(parameters), |services| {
        services
            .register(ProximityServiceHandle::new(service))
            .unwrap();
    })
}

fn walls() -> Stub {
    Stub::default()
        .at("pipe", 0.0, 0.0)
        .at("near", 1.5, 0.0)
        .at("mid", 1.8, 0.0)
        .at("far", 4.0, 0.0)
        .distance("pipe", "near", "Minimum3d", (0.5, 0.5))
        .distance("pipe", "mid", "Minimum3d", (0.8, 0.8))
        .distance("pipe", "far", "Minimum3d", (3.0, 3.0))
}

fn at_least(count: i64, maximum: f64) -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("mode", string("at_least")),
        ("count", integer(count)),
        ("maximum_metres", number(maximum)),
    ]
}

fn only_finding(outcome: &CapabilityEvaluation) -> &axioval_ir::Finding {
    let [finding] = outcome.findings() else {
        panic!(
            "one finding expected: {:?} / {:?}",
            outcome.findings(),
            outcome.not_evaluated_outcomes()
        );
    };
    finding
}

fn reasons(outcome: &CapabilityEvaluation) -> Vec<(String, NotEvaluatedReason)> {
    common::unevaluated(outcome)
}

#[test]
fn at_least_counts_counterparts_within_the_maximum() {
    let outcome = run(model(), walls(), at_least(2, 1.0));
    assert!(outcome.findings().is_empty() && outcome.not_evaluated_outcomes().is_empty());

    let outcome = run(model(), walls(), at_least(3, 1.0));
    let finding = only_finding(&outcome);
    assert!(
        finding
            .message
            .contains("2 counterpart(s) lie within 1.0000 m, 3 required"),
        "{}",
        finding.message
    );
    assert_eq!(finding.related, vec![id("mid"), id("near")]);
}

/// N within a range: a counterpart nearer than the minimum does not count.
#[test]
fn at_least_within_a_range_skips_counterparts_too_close() {
    let mut parameters = at_least(2, 1.0);
    parameters.push(("minimum_metres", number(0.6)));
    let outcome = run(model(), walls(), parameters);
    let finding = only_finding(&outcome);
    assert!(finding.message.contains("between 0.6000 and 1.0000 m"));
    assert_eq!(finding.related, vec![id("mid")]);
}

/// A straddling interval might or might not count: judge only when it
/// cannot change the verdict.
#[test]
fn an_undecided_counterpart_counts_as_unknown() {
    let stub = || {
        Stub::default()
            .at("pipe", 0.0, 0.0)
            .at("near", 1.5, 0.0)
            .curved("mid", 1.8, 0.0)
            .at("far", 4.0, 0.0)
            .distance("pipe", "near", "Minimum3d", (0.5, 0.5))
            .distance("pipe", "mid", "Minimum3d", (0.95, 1.05))
            .distance("pipe", "far", "Minimum3d", (3.0, 3.0))
    };
    let outcome = run(model(), stub(), at_least(1, 1.0));
    assert!(outcome.findings().is_empty() && outcome.not_evaluated_outcomes().is_empty());

    let outcome = run(model(), stub(), at_least(2, 1.0));
    assert!(outcome.findings().is_empty());
    let [(object, reason)] = &reasons(&outcome)[..] else {
        panic!("one outcome expected");
    };
    assert_eq!(
        (object.as_str(), reason),
        ("pipe", &NotEvaluatedReason::IncompleteEvidence)
    );
    assert!(
        outcome.not_evaluated_outcomes()[0]
            .message()
            .contains("straddles")
    );

    // Even counting it, three cannot be reached.
    let outcome = run(model(), stub(), at_least(3, 1.0));
    assert!(only_finding(&outcome).message.contains("3 required"));
}

/// An unreadable counterpart might be one of the N: a shortfall it could fill
/// is not evaluated, while one it could not fill stands.
#[test]
fn an_unmeasured_counterpart_counts_as_unknown() {
    let stub = || {
        Stub::default()
            .at("pipe", 0.0, 0.0)
            .at("near", 1.5, 0.0)
            .without_geometry("mid")
            .at("far", 4.0, 0.0)
            .distance("pipe", "near", "Minimum3d", (0.5, 0.5))
    };
    let outcome = run(model(), stub(), at_least(2, 1.0));
    assert!(outcome.findings().is_empty());
    let mut objects: Vec<String> = reasons(&outcome).into_iter().map(|(o, _)| o).collect();
    objects.sort();
    assert_eq!(objects, vec!["mid", "pipe"]);

    let outcome = run(model(), stub(), at_least(3, 1.0));
    assert!(only_finding(&outcome).message.contains("2 counterpart(s)"));
}

#[test]
fn none_closer_than_names_every_counterpart_too_close() {
    let outcome = run(
        model(),
        walls(),
        vec![
            ("mode", string("none_closer_than")),
            ("minimum_metres", number(1.0)),
        ],
    );
    let finding = only_finding(&outcome);
    assert!(
        finding
            .message
            .starts_with("2 counterpart(s) closer than the required 1.0000 m; nearest"),
        "{}",
        finding.message
    );
    assert_eq!(finding.related, vec![id("mid"), id("near")]);
    assert_eq!(finding.evidence.len(), 2);

    let outcome = run(
        model(),
        walls(),
        vec![
            ("mode", string("none_closer_than")),
            ("minimum_metres", number(0.4)),
        ],
    );
    assert!(outcome.findings().is_empty() && outcome.not_evaluated_outcomes().is_empty());
}

/// A tessellated counterpart straddling the minimum leaves it open; a
/// certain violation elsewhere stands regardless.
#[test]
fn none_closer_than_is_open_only_while_nothing_certainly_violates() {
    let stub = |near: (f64, f64)| {
        Stub::default()
            .at("pipe", 0.0, 0.0)
            .at("near", 1.5, 0.0)
            .curved("mid", 1.5, 0.0)
            .at("far", 4.0, 0.0)
            .distance("pipe", "near", "Minimum3d", near)
            .distance("pipe", "mid", "Minimum3d", (0.58, 0.62))
    };
    let parameters = || {
        vec![
            ("mode", string("none_closer_than")),
            ("minimum_metres", number(0.6)),
        ]
    };
    let outcome = run(model(), stub((0.7, 0.7)), parameters());
    assert!(outcome.findings().is_empty());
    assert!(
        outcome.not_evaluated_outcomes()[0]
            .message()
            .contains("straddles")
    );

    let outcome = run(model(), stub((0.5, 0.5)), parameters());
    assert_eq!(only_finding(&outcome).related, vec![id("near")]);
}

/// Bodies on different storeys: the broad phase in space would drop the pair,
/// in plan it must not.
#[test]
fn horizontal_projection_measures_in_plan() {
    let stub = Stub::default()
        .at("pipe", 0.0, 0.0)
        .at("near", 1.5, 10.0)
        .at("mid", 9.0, 10.0)
        .at("far", 20.0, 0.0)
        .distance("pipe", "near", "Horizontal", (0.5, 0.5));
    let mut parameters = at_least(1, 1.0);
    parameters.push(("projection", string("horizontal")));
    let outcome = run(model(), stub, parameters);
    assert!(outcome.findings().is_empty() && outcome.not_evaluated_outcomes().is_empty());
}

/// Only bodies above or below count; one beside the subject has no vertical
/// distance at all.
#[test]
fn vertical_projection_counts_only_bodies_above_or_below() {
    let stub = || {
        Stub::default()
            .at("pipe", 0.0, 0.0)
            .at("near", 0.5, 3.0)
            .at("mid", 0.9, 1.5)
            .at("far", 20.0, 0.0)
            .distance(
                "pipe",
                "near",
                "Vertical { footprint_offset_metres: 0.0 }",
                (2.0, 2.0),
            )
            .distance(
                "pipe",
                "mid",
                "Vertical { footprint_offset_metres: 0.0 }",
                (f64::INFINITY, f64::INFINITY),
            )
    };
    let parameters = |count| {
        let mut parameters = at_least(count, 3.0);
        parameters.push(("projection", string("vertical")));
        parameters
    };
    let outcome = run(model(), stub(), parameters(1));
    assert!(outcome.findings().is_empty() && outcome.not_evaluated_outcomes().is_empty());
    let outcome = run(model(), stub(), parameters(2));
    assert!(only_finding(&outcome).message.contains("1 counterpart(s)"));
}

/// The footprint offset reaches the measurement unchanged, and widens the
/// broad phase in plan.
#[test]
fn a_footprint_offset_reaches_the_service() {
    let stub = Stub::default()
        .at("pipe", 0.0, 0.0)
        .at("near", 1.4, 3.0)
        .at("mid", 9.0, 0.0)
        .at("far", 20.0, 0.0)
        .distance(
            "pipe",
            "near",
            "Vertical { footprint_offset_metres: 0.5 }",
            (2.0, 2.0),
        );
    let mut parameters = at_least(1, 3.0);
    parameters.push(("projection", string("vertical")));
    parameters.push(("footprint_offset_metres", number(0.5)));
    let outcome = run(model(), stub, parameters);
    assert!(outcome.findings().is_empty() && outcome.not_evaluated_outcomes().is_empty());
}

/// Nothing may overlap the subject in plan; a tessellated overlap left open
/// is not evaluated.
#[test]
fn plan_overlap_projection_finds_overlapping_footprints() {
    let stub = |mid: (f64, f64)| {
        Stub::default()
            .at("pipe", 0.0, 0.0)
            .at("near", 0.5, 5.0)
            .curved("mid", 0.9, -5.0)
            .at("far", 20.0, 0.0)
            .distance("pipe", "near", "PlanOverlap", (0.0, 0.0))
            .distance("pipe", "mid", "PlanOverlap", mid)
    };
    let parameters = || {
        vec![
            ("mode", string("none_closer_than")),
            ("minimum_metres", number(0.001)),
            ("projection", string("plan_overlap")),
        ]
    };
    let outcome = run(model(), stub((f64::INFINITY, f64::INFINITY)), parameters());
    let finding = only_finding(&outcome);
    assert_eq!(finding.related, vec![id("near")]);
    assert!(finding.message.contains("plan-overlap distance 0.0000 m"));

    let only_open = Stub::default()
        .at("pipe", 0.0, 0.0)
        .at("near", 5.0, 5.0)
        .curved("mid", 0.9, -5.0)
        .at("far", 20.0, 0.0)
        .distance("pipe", "mid", "PlanOverlap", (0.0, f64::INFINITY));
    let outcome = run(model(), only_open, parameters());
    assert!(outcome.findings().is_empty());
    assert_eq!(
        reasons(&outcome),
        vec![("pipe".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

/// Counterparts in another space do not count; the subject's own space is
/// reached through the declared relationship.
#[test]
fn counterparts_are_scoped_to_the_subjects_container() {
    let scoped = || {
        model()
            .object("kitchen", "space")
            .object("hall", "space")
            .edge("contains", "kitchen", "pipe")
            .edge("contains", "kitchen", "mid")
            .edge("contains", "hall", "near")
    };
    let parameters = |count| {
        let mut parameters = at_least(count, 1.0);
        parameters.push(("path", strings(&["contains:backward"])));
        parameters
    };
    let outcome = run(scoped(), walls(), parameters(1));
    assert!(outcome.findings().is_empty() && outcome.not_evaluated_outcomes().is_empty());
    let outcome = run(scoped(), walls(), parameters(2));
    let finding = only_finding(&outcome);
    assert_eq!(finding.related, vec![id("mid")]);
    assert!(
        finding
            .evidence
            .iter()
            .any(|evidence| evidence.locator == "scan:contains"),
        "the scope's evidence is cited"
    );

    // Out of scope, the near wall no longer breaks a minimum either.
    let mut apart = parameters(1);
    apart.retain(|(name, _)| *name == "path");
    apart.extend([
        ("mode", string("none_closer_than")),
        ("minimum_metres", number(0.6)),
    ]);
    let outcome = run(scoped(), walls(), apart);
    assert!(outcome.findings().is_empty() && outcome.not_evaluated_outcomes().is_empty());
}

#[test]
fn an_undecided_container_leaves_the_subject_not_evaluated() {
    let mut parameters = at_least(1, 1.0);
    parameters.push(("path", strings(&["unknown-relationship"])));
    let outcome = run(model(), walls(), parameters);
    assert!(outcome.findings().is_empty());
    assert_eq!(
        reasons(&outcome),
        vec![("pipe".to_owned(), NotEvaluatedReason::BackendUnavailable)]
    );
}

#[test]
fn a_service_without_projections_fails_closed() {
    let mut parameters = at_least(1, 1.0);
    parameters.push(("projection", string("horizontal")));
    let outcome = run(model(), SpaceOnly, parameters);
    assert!(outcome.findings().is_empty());
    assert_eq!(
        reasons(&outcome),
        vec![("pipe".to_owned(), NotEvaluatedReason::BackendUnavailable)]
    );
    // In space the default method answers from the full measurement.
    let outcome = run(model(), SpaceOnly, at_least(3, 1.0));
    assert!(outcome.findings().is_empty() && outcome.not_evaluated_outcomes().is_empty());
}

#[test]
fn invalid_mode_projection_and_count_declarations_are_refused() {
    let declarations: Vec<Vec<(&str, ParameterValue)>> = vec![
        vec![("mode", string("most")), ("maximum_metres", number(1.0))],
        vec![
            ("mode", string("at_least")),
            ("maximum_metres", number(1.0)),
        ],
        vec![("mode", string("at_least")), ("count", integer(2))],
        at_least(0, 1.0),
        vec![("count", integer(2)), ("maximum_metres", number(1.0))],
        vec![
            ("mode", string("none_closer_than")),
            ("minimum_metres", number(0.5)),
            ("maximum_metres", number(1.0)),
        ],
        vec![
            ("mode", string("none_closer_than")),
            ("minimum_metres", number(0.0)),
        ],
        vec![
            ("maximum_metres", number(1.0)),
            ("footprint_offset_metres", number(0.5)),
        ],
        vec![
            ("maximum_metres", number(1.0)),
            ("projection", string("diagonal")),
        ],
        vec![
            ("maximum_metres", number(1.0)),
            ("projection", string("vertical")),
            ("footprint_offset_metres", number(-0.5)),
        ],
        {
            let mut range = at_least(1, 1.0);
            range.push(("minimum_metres", number(2.0)));
            range
        },
    ];
    for parameters in declarations {
        let outcome = run(model(), Stub::default(), parameters);
        assert_eq!(
            reasons(&outcome),
            vec![("pipe".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

/// Every declared parameter is part of the signature a definition binds.
#[test]
fn the_signature_declares_modes_projections_and_scoping() {
    let parameters = Distance.parameters();
    let names: Vec<&str> = parameters.iter().map(|p| p.name.as_str()).collect();
    for expected in [
        "mode",
        "count",
        "projection",
        "footprint_offset_metres",
        "relationship",
        "path",
    ] {
        assert!(names.contains(&expected), "{expected} missing");
    }
    let count = Distance
        .parameters()
        .into_iter()
        .find(|p| p.name == "count")
        .unwrap();
    assert_eq!(count.parameter_type.package_kind(), "integer");
}
