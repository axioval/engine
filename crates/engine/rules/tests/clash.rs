//! Clash and distance capability contract tests.
//!
//! ADR 0004: the service measures, the capability decides. The stub below
//! answers only the pairs a test declares and panics on any other, which also
//! proves the broad phase kept far-apart pairs away from the narrow phase.
#![allow(missing_docs)]

use std::{collections::BTreeMap, sync::Arc};

use axioval_engine::{
    BodyContainment, Bounds3, CapabilityEvaluation, CompiledRule, GeometryFidelity,
    NotEvaluatedReason, ObjectBounds, ProximityError, ProximityEvidence, ProximityRequest,
    ProximityService, ProximityServiceHandle, RuleCapability, RuleContext, ServiceRegistry,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity as RuleSeverity};
use axioval_ir::{Evidence, Object, ObjectId, Project, RuleId, SourceId};
use axioval_rules::{Clash, Distance};

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}
fn oid(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

#[derive(Clone, Copy)]
struct Pair {
    separation: f64,
    penetration: Option<f64>,
    containment: Option<BodyContainment>,
}
fn apart(separation: f64) -> Pair {
    Pair {
        separation,
        penetration: Some(0.0),
        containment: None,
    }
}
fn overlapping(depth: f64) -> Pair {
    Pair {
        separation: 0.0,
        penetration: Some(depth),
        containment: None,
    }
}

#[derive(Default)]
struct Stub {
    /// x-offset of a unit box per object; `None` means no geometry.
    boxes: BTreeMap<String, Option<(f64, GeometryFidelity)>>,
    pairs: BTreeMap<(String, String), Result<Pair, ProximityError>>,
}

impl Stub {
    fn object(mut self, local: &str, x: f64) -> Self {
        self.boxes
            .insert(local.into(), Some((x, GeometryFidelity::Exact)));
        self
    }
    fn tessellated(mut self, local: &str, x: f64) -> Self {
        self.boxes.insert(
            local.into(),
            Some((x, GeometryFidelity::tessellated(0.002).unwrap())),
        );
        self
    }
    fn without_geometry(mut self, local: &str) -> Self {
        self.boxes.insert(local.into(), None);
        self
    }
    fn pair(mut self, a: &str, b: &str, pair: Pair) -> Self {
        self.pairs.insert((a.into(), b.into()), Ok(pair));
        self
    }
    fn failing(mut self, a: &str, b: &str) -> Self {
        self.pairs
            .insert((a.into(), b.into()), Err(ProximityError::Unavailable));
        self
    }
    fn fidelity(&self, local: &str) -> GeometryFidelity {
        self.boxes[local].unwrap().1
    }
}

impl ProximityService for Stub {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        let (x, fidelity) = self
            .boxes
            .get(&object.local_id)
            .copied()
            .flatten()
            .ok_or(ProximityError::Unavailable)?;
        ObjectBounds::try_new(
            object.clone(),
            Bounds3::try_new([x, 0.0, 0.0], [x + 1.0, 1.0, 1.0])?,
            fidelity,
        )
    }

    fn measure_proximity(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProximityEvidence, ProximityError> {
        let (a, b) = (
            request.subject().local_id.clone(),
            request.counterpart().local_id.clone(),
        );
        let pair = *self
            .pairs
            .get(&(a.clone(), b.clone()))
            .or_else(|| self.pairs.get(&(b.clone(), a.clone())))
            .unwrap_or_else(|| panic!("broad phase should have pruned {a}/{b}"));
        let pair = pair?;
        let fidelity = self.fidelity(&a).combined(self.fidelity(&b));
        ProximityEvidence::try_new(
            request.clone(),
            pair.separation,
            pair.penetration,
            0.0,
            pair.containment,
            fidelity,
            Evidence {
                source: source(),
                locator: format!("proximity:{a}:{b}"),
                exact: fidelity.is_exact(),
            },
        )
    }
}

fn kind(object_type: &str) -> Selector {
    Selector::EntityType {
        object_type: object_type.into(),
        include_subtypes: false,
    }
}

fn rule(capability: &str, subjects: Selector, parameters: &[(&str, f64)]) -> CompiledRule {
    let mut bound = BTreeMap::from([(
        "counterparts".to_string(),
        ParameterValue::Selector {
            value: Box::new(kind("wall")),
        },
    )]);
    for (name, value) in parameters {
        bound.insert(
            (*name).to_string(),
            ParameterValue::Number { value: *value },
        );
    }
    CompiledRule {
        id: RuleId::new("check").unwrap(),
        capability: capability.into(),
        severity: RuleSeverity::Error,
        selector: subjects,
        parameters: bound,
    }
}

fn clash(parameters: &[(&str, f64)]) -> CompiledRule {
    rule("axioval:capability.clash", kind("pipe"), parameters)
}

fn distance(parameters: &[(&str, f64)]) -> CompiledRule {
    rule("axioval:capability.distance", kind("pipe"), parameters)
}

fn project(objects: &[(&str, &str)]) -> Project {
    Project::new(
        objects
            .iter()
            .map(|(local, kind)| Object::new(oid(local), *kind))
            .collect(),
    )
    .unwrap()
}

fn run(
    capability: &dyn RuleCapability,
    project: &Project,
    stub: Stub,
    rule: &CompiledRule,
) -> CapabilityEvaluation {
    let mut services = ServiceRegistry::new();
    services
        .register(ProximityServiceHandle::new(Arc::new(stub)))
        .unwrap();
    capability.evaluate(
        &RuleContext {
            project,
            services: &services,
        },
        rule,
    )
}

fn pipes_and_walls() -> Project {
    project(&[("pipe", "pipe"), ("wall", "wall"), ("far-wall", "wall")])
}

#[test]
fn penetration_beyond_tolerance_is_a_hard_clash_naming_the_counterpart() {
    let stub = Stub::default()
        .object("pipe", 0.0)
        .object("wall", 0.5)
        .object("far-wall", 50.0)
        .pair("pipe", "wall", overlapping(0.1));
    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        stub,
        &clash(&[("penetration_tolerance_metres", 0.01)]),
    );
    assert!(outcome.not_evaluated_outcomes().is_empty());
    let [finding] = outcome.findings() else {
        panic!("one clash expected: {:?}", outcome.findings());
    };
    assert_eq!(finding.object_id, oid("pipe"));
    assert_eq!(finding.related, vec![oid("wall")]);
    assert!(
        finding.message.starts_with("hard clash"),
        "{}",
        finding.message
    );
    assert!(finding.evidence[0].exact);
}

/// Zero separation is how a slab rests on a wall. Touching is not clashing.
#[test]
fn touching_within_tolerance_is_not_a_clash() {
    let stub = Stub::default()
        .object("pipe", 0.0)
        .object("wall", 1.0)
        .object("far-wall", 50.0)
        .pair("pipe", "wall", overlapping(0.0));
    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        stub,
        &clash(&[("penetration_tolerance_metres", 0.0)]),
    );
    assert!(outcome.findings().is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

#[test]
fn a_body_inside_another_clashes_although_the_surfaces_are_apart() {
    let stub = Stub::default()
        .object("pipe", 0.0)
        .object("wall", 0.2)
        .object("far-wall", 50.0)
        .pair(
            "pipe",
            "wall",
            Pair {
                separation: 0.2,
                penetration: Some(0.3),
                containment: Some(BodyContainment::SubjectInsideCounterpart),
            },
        );
    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        stub,
        &clash(&[("penetration_tolerance_metres", 0.5)]),
    );
    assert_eq!(outcome.findings().len(), 1);
    assert!(outcome.findings()[0].message.contains("wholly inside"));
}

#[test]
fn coming_closer_than_the_clearance_is_a_clearance_clash() {
    let stub = Stub::default()
        .object("pipe", 0.0)
        .object("wall", 1.05)
        .object("far-wall", 50.0)
        .pair("pipe", "wall", apart(0.05));
    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        stub,
        &clash(&[
            ("penetration_tolerance_metres", 0.0),
            ("clearance_metres", 0.1),
        ]),
    );
    assert_eq!(outcome.findings().len(), 1);
    assert!(outcome.findings()[0].message.starts_with("clearance clash"));
}

/// An open surface has no inside. Meeting surfaces could be touching or
/// crossing, so the pair is reported unevaluated -- never passed.
#[test]
fn meeting_surfaces_without_a_penetration_measurement_are_not_evaluated() {
    let stub = Stub::default()
        .object("pipe", 0.0)
        .object("wall", 1.0)
        .object("far-wall", 50.0)
        .pair(
            "pipe",
            "wall",
            Pair {
                separation: 0.0,
                penetration: None,
                containment: None,
            },
        );
    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        stub,
        &clash(&[("penetration_tolerance_metres", 0.01)]),
    );
    assert!(outcome.findings().is_empty());
    let [refused] = outcome.not_evaluated_outcomes() else {
        panic!("one refusal expected");
    };
    assert_eq!(refused.object_id(), Some(&oid("pipe")));
    assert_eq!(refused.reason(), &NotEvaluatedReason::IncompleteEvidence);
}

#[test]
fn tessellated_clashes_are_reported_as_approximate() {
    let stub = Stub::default()
        .tessellated("pipe", 0.0)
        .object("wall", 0.5)
        .object("far-wall", 50.0)
        .pair("pipe", "wall", overlapping(0.1));
    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        stub,
        &clash(&[("penetration_tolerance_metres", 0.01)]),
    );
    let [finding] = outcome.findings() else {
        panic!("one clash expected");
    };
    assert!(
        !finding.evidence[0].exact,
        "tessellated evidence is not exact"
    );
    assert!(
        finding.message.contains("approximate"),
        "{}",
        finding.message
    );
}

/// Without a counterpart's extent, a clash against it is invisible. The
/// report must name it rather than read as clean.
#[test]
fn a_counterpart_without_geometry_is_reported_not_skipped() {
    let stub = Stub::default()
        .object("pipe", 0.0)
        .without_geometry("wall")
        .object("far-wall", 50.0);
    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        stub,
        &clash(&[("penetration_tolerance_metres", 0.01)]),
    );
    assert!(outcome.findings().is_empty());
    let [refused] = outcome.not_evaluated_outcomes() else {
        panic!("one refusal expected");
    };
    assert_eq!(refused.object_id(), Some(&oid("wall")));
}

#[test]
fn a_group_checked_against_itself_reports_each_pair_once() {
    let project = project(&[("a", "wall"), ("b", "wall")]);
    let stub = Stub::default()
        .object("a", 0.0)
        .object("b", 0.5)
        .pair("a", "b", overlapping(0.2));
    let rule = rule(
        "axioval:capability.clash",
        kind("wall"),
        &[("penetration_tolerance_metres", 0.01)],
    );
    let outcome = run(&Clash, &project, stub, &rule);
    assert_eq!(outcome.findings().len(), 1);
    assert_eq!(outcome.findings()[0].object_id, oid("a"));
}

#[test]
fn missing_service_and_invalid_declarations_are_not_evaluated() {
    let project = pipes_and_walls();
    let services = ServiceRegistry::new();
    let outcome = Clash.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        &clash(&[("penetration_tolerance_metres", 0.01)]),
    );
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::MissingService
    );

    for invalid in [
        clash(&[("penetration_tolerance_metres", -1.0)]),
        clash(&[]),
        distance(&[]),
        distance(&[("minimum_metres", 2.0), ("maximum_metres", 1.0)]),
    ] {
        let capability: &dyn RuleCapability = if invalid.capability.ends_with("clash") {
            &Clash
        } else {
            &Distance
        };
        let outcome = run(capability, &project, Stub::default(), &invalid);
        assert_eq!(
            outcome.not_evaluated_outcomes()[0].reason(),
            &NotEvaluatedReason::InvalidDeclaration
        );
    }
}

#[test]
fn nothing_within_the_maximum_is_a_distance_finding() {
    let stub = Stub::default()
        .object("pipe", 0.0)
        .object("wall", 10.0)
        .object("far-wall", 50.0);
    let outcome = run(
        &Distance,
        &pipes_and_walls(),
        stub,
        &distance(&[("maximum_metres", 2.0)]),
    );
    let [finding] = outcome.findings() else {
        panic!("one finding expected");
    };
    assert!(finding.message.contains("no counterpart lies within"));
    assert!(finding.related.is_empty());
}

#[test]
fn the_nearest_counterpart_decides_the_maximum() {
    let stub = Stub::default()
        .object("pipe", 0.0)
        .object("wall", 1.5)
        .object("far-wall", 2.5)
        .pair("pipe", "wall", apart(0.5))
        .pair("pipe", "far-wall", apart(1.5));
    let outcome = run(
        &Distance,
        &pipes_and_walls(),
        stub,
        &distance(&[("maximum_metres", 1.0)]),
    );
    assert!(outcome.findings().is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

#[test]
fn a_counterpart_nearer_than_the_minimum_is_a_distance_finding() {
    let stub = Stub::default()
        .object("pipe", 0.0)
        .object("wall", 1.2)
        .object("far-wall", 50.0)
        .pair("pipe", "wall", apart(0.2));
    let outcome = run(
        &Distance,
        &pipes_and_walls(),
        stub,
        &distance(&[("minimum_metres", 0.5)]),
    );
    let [finding] = outcome.findings() else {
        panic!("one finding expected");
    };
    assert_eq!(finding.related, vec![oid("wall")]);
    assert!(finding.message.contains("closer than"));
}

/// The unreadable counterpart might be the near one, so "too far" cannot be
/// concluded. Once a measured counterpart meets the maximum, it can.
#[test]
fn an_unmeasured_counterpart_blocks_only_an_unmet_maximum() {
    let unmet = Stub::default()
        .object("pipe", 0.0)
        .without_geometry("wall")
        .object("far-wall", 50.0);
    let outcome = run(
        &Distance,
        &pipes_and_walls(),
        unmet,
        &distance(&[("maximum_metres", 2.0)]),
    );
    assert!(outcome.findings().is_empty());
    let reported: Vec<_> = outcome
        .not_evaluated_outcomes()
        .iter()
        .filter_map(|outcome| outcome.object_id().cloned())
        .collect();
    assert_eq!(reported, vec![oid("pipe"), oid("wall")]);

    let met = Stub::default()
        .object("pipe", 0.0)
        .without_geometry("wall")
        .object("far-wall", 1.5)
        .pair("pipe", "far-wall", apart(0.5));
    let outcome = run(
        &Distance,
        &pipes_and_walls(),
        met,
        &distance(&[("maximum_metres", 2.0)]),
    );
    assert!(outcome.findings().is_empty());
    let reported: Vec<_> = outcome
        .not_evaluated_outcomes()
        .iter()
        .filter_map(|outcome| outcome.object_id().cloned())
        .collect();
    assert_eq!(reported, vec![oid("wall")], "only the unreadable object");
}

#[test]
fn a_failed_measurement_is_not_a_pass() {
    let stub = Stub::default()
        .object("pipe", 0.0)
        .object("wall", 1.2)
        .object("far-wall", 50.0)
        .failing("pipe", "wall");
    let outcome = run(
        &Distance,
        &pipes_and_walls(),
        stub,
        &distance(&[("minimum_metres", 0.5)]),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].object_id(),
        Some(&oid("pipe"))
    );
}
