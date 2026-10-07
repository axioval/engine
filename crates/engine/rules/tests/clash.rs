//! Clash and distance capability contract tests.
//!
//! ADR 0004: the service measures, the capability decides. The stub below
//! answers only the pairs a test declares and panics on any other, which also
//! proves the broad phase kept far-apart pairs away from the narrow phase.
#![allow(missing_docs)]

use std::{collections::BTreeMap, sync::Arc};

use axioval_engine::{
    BodyContainment, Bounds3, CapabilityEvaluation, CapabilityRegistry, CompiledRule,
    GeometryFidelity, IntersectionVolume, LengthInterval, NotEvaluatedReason, ObjectBounds,
    OverlapExtents, ProximityError, ProximityEvidence, ProximityRequest, ProximityService,
    ProximityServiceHandle, RuleCapability, RuleContext, ServiceRegistry, VolumeInterval,
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

#[derive(Clone, Copy, Debug)]
struct Pair {
    separation: f64,
    penetration: Option<f64>,
    containment: Option<BodyContainment>,
    /// Hausdorff distance bounds; distinct bodies by default.
    hausdorff: Option<(f64, f64)>,
    /// `(lower, upper)` of the x, y and z extents of the intersection.
    extents: Option<[(f64, f64); 3]>,
    /// `(lower, upper)` of the shared volume, the bodies 1 m³ each.
    volume: Option<(f64, f64)>,
    /// `(lower, upper)` of a certified separation on exact boundaries.
    certified: Option<(f64, f64)>,
}
fn apart(separation: f64) -> Pair {
    Pair {
        separation,
        penetration: Some(0.0),
        containment: None,
        hausdorff: Some((separation.max(1.0), separation.max(1.0))),
        extents: None,
        volume: None,
        certified: None,
    }
}
fn overlapping(depth: f64) -> Pair {
    Pair {
        separation: 0.0,
        penetration: Some(depth),
        containment: None,
        hausdorff: Some((1.0, 1.0)),
        extents: None,
        volume: None,
        certified: None,
    }
}
impl Pair {
    fn hausdorff(self, lower: f64, upper: f64) -> Self {
        Self {
            hausdorff: Some((lower, upper)),
            ..self
        }
    }
    fn unmeasured_hausdorff(self) -> Self {
        Self {
            hausdorff: None,
            ..self
        }
    }
    fn volume(self, lower: f64, upper: f64) -> Self {
        Self {
            volume: Some((lower, upper)),
            ..self
        }
    }
    fn certified(self, lower: f64, upper: f64) -> Self {
        Self {
            certified: Some((lower, upper)),
            ..self
        }
    }
    fn extents(self, extents: [(f64, f64); 3]) -> Self {
        Self {
            extents: Some(extents),
            ..self
        }
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
    fn refused(mut self, a: &str, b: &str, reason: &'static str) -> Self {
        self.pairs
            .insert((a.into(), b.into()), Err(ProximityError::Refused(reason)));
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
        let measured = ProximityEvidence::try_new(
            request.clone(),
            pair.separation,
            pair.penetration,
            Some(0.0),
            pair.containment,
            fidelity,
            Evidence {
                source: source(),
                locator: format!("proximity:{a}:{b}"),
                exact: fidelity.is_exact(),
            },
        )?;
        let measured = match pair.certified {
            Some((lower, upper)) => measured
                .with_certified_separation(LengthInterval::try_new(lower, upper).unwrap())?,
            None => measured,
        };
        let measured = match pair.hausdorff {
            Some((lower, upper)) => {
                measured.with_hausdorff(LengthInterval::try_new(lower, upper).unwrap())?
            }
            None => measured,
        };
        let measured = match pair.extents {
            Some(axes) => {
                let [along_x, along_y, along_z] =
                    axes.map(|(lower, upper)| LengthInterval::try_new(lower, upper).unwrap());
                measured.with_overlap_extents(OverlapExtents::new(along_x, along_y, along_z))?
            }
            None => measured,
        };
        match pair.volume {
            Some((lower, upper)) => {
                let body = VolumeInterval::exact(1.0).unwrap();
                measured.with_intersection_volume(IntersectionVolume::try_new(
                    VolumeInterval::try_new(lower, upper).unwrap(),
                    body,
                    body,
                )?)
            }
            None => Ok(measured),
        }
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

/// `capability`'s evaluation of `rule`, its measured values installed as a
/// run installs them; `clash`, which runs as a template, held to the
/// implementation it replaced under the whole outside contract.
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
    // `clash` and `distance` run as templates over measured lists, each
    // held to the implementation it replaced under the parity contract.
    let registry =
        axioval_rules::register_builtins(axioval_engine::CapabilityRegistry::new()).unwrap();
    let mut inner = services.clone();
    registry.install_measured(&mut inner, project);
    let values = axioval_engine::MeasuredValues::of(&inner, project);
    registry.install_measured(&mut services, project);
    services.register(values).unwrap();
    let context = RuleContext {
        project,
        services: &services,
    };
    let template = capability.evaluate(&context, rule);
    let reference: &dyn RuleCapability = match capability.id() {
        "axioval:capability.distance" => &axioval_rules::reference::Distance,
        "axioval:capability.clash" => &axioval_rules::reference::Clash,
        _ => return template,
    };
    let reference = reference.evaluate(&context, rule);
    let parity = axioval_rules::parity::Parity::contract().compare(
        (
            capability.id(),
            &axioval_rules::parity::Observations::of_evaluation(&reference),
        ),
        (
            "template",
            &axioval_rules::parity::Observations::of_evaluation(&template),
        ),
    );
    assert!(parity.holds(), "{}", parity.diff());
    template
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
    assert_eq!(finding.object_id(), Some(&oid("pipe")));
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
                hausdorff: Some((0.2, 1.0)),
                extents: None,
                volume: None,
                certified: None,
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

/// A tessellated pair with a certified separation is judged on that
/// interval: decided where it clears the clearance, open where it straddles
/// it, whatever the mesh separation says.
#[test]
fn a_certified_separation_is_judged_as_an_interval() {
    let judge = |certified: (f64, f64)| {
        let stub = Stub::default()
            .tessellated("pipe", 0.0)
            .object("wall", 1.05)
            .object("far-wall", 50.0)
            // The mesh separation alone would read as a pass.
            .pair(
                "pipe",
                "wall",
                apart(0.101).certified(certified.0, certified.1),
            );
        run(
            &Clash,
            &pipes_and_walls(),
            stub,
            &clash(&[
                ("penetration_tolerance_metres", 0.0),
                ("clearance_metres", 0.1),
            ]),
        )
    };
    let below = judge((0.099_999, 0.099_999_5));
    let [finding] = below.findings() else {
        panic!("one clearance clash expected");
    };
    assert!(
        finding.message.starts_with("clearance clash"),
        "{}",
        finding.message
    );
    assert!(finding.message.contains("certified"), "{}", finding.message);

    let above = judge((0.100_001, 0.100_002));
    assert!(above.findings().is_empty());
    assert!(above.not_evaluated_outcomes().is_empty());

    let straddling = judge((0.099_999, 0.100_001));
    assert!(straddling.findings().is_empty());
    let [open] = straddling.not_evaluated_outcomes() else {
        panic!("one open pair expected");
    };
    assert_eq!(open.reason(), &NotEvaluatedReason::IncompleteEvidence);
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
                hausdorff: Some((0.5, 0.5)),
                extents: None,
                volume: None,
                certified: None,
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

/// A pair the proximity service measured on tessellated geometry is never
/// exact, however its numbers look; one measured exactly is.
#[test]
fn pairs_measured_on_tessellated_geometry_are_inexact() {
    let stub = Stub::default()
        .tessellated("pipe", 0.0)
        .object("wall", 0.5)
        .object("far-wall", 50.0)
        .object("duct", 49.5)
        .pair("pipe", "wall", overlapping(0.1))
        .pair("duct", "far-wall", overlapping(0.1));
    let project = project(&[
        ("pipe", "pipe"),
        ("duct", "pipe"),
        ("wall", "wall"),
        ("far-wall", "wall"),
    ]);
    let mut services = ServiceRegistry::new();
    services
        .register(ProximityServiceHandle::new(Arc::new(stub)))
        .unwrap();
    axioval_rules::register_builtins(CapabilityRegistry::new())
        .unwrap()
        .install_measured(&mut services, &project);
    let pairs = axioval_engine::measured_members(
        &services,
        &oid("pipe"),
        "clash_pairs;subjects=pipe;counterparts=wall;penetration_tolerance_metres=0.01",
    )
    .unwrap();
    let exactness: Vec<(String, bool)> = pairs
        .iter()
        .map(|pair| {
            let Some(axioval_engine::MemberValue::Objects { objects }) = pair.fields.get("subject")
            else {
                panic!("a pair names its subject");
            };
            (objects[0].local_id.clone(), pair.exact)
        })
        .collect();
    assert_eq!(
        exactness,
        [("duct".to_owned(), true), ("pipe".to_owned(), false)]
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
    assert_eq!(outcome.findings()[0].object_id(), Some(&oid("a")));
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

/// A kernel refusing a measured pair is not evaluated, and the outcome says
/// why, never that the object has nothing to measure.
#[test]
fn a_kernel_refusal_is_not_evaluated_with_its_reason() {
    let stub = Stub::default()
        .object("pipe", 0.0)
        .object("wall", 1.0)
        .object("far-wall", 50.0)
        .refused("pipe", "wall", "the plan overlay refused the footprints");
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
    assert!(
        refused.message().ends_with(
            "the geometry kernel refused the measurement: the plan overlay refused the footprints"
        ),
        "{}",
        refused.message()
    );
    assert!(
        !refused
            .message()
            .contains("unavailable for the requested object")
    );
}

fn one_message(outcome: &CapabilityEvaluation) -> &str {
    assert!(
        outcome.not_evaluated_outcomes().is_empty(),
        "{:?}",
        outcome.not_evaluated_outcomes()
    );
    let [finding] = outcome.findings() else {
        panic!("one finding expected: {:?}", outcome.findings());
    };
    &finding.message
}

fn walls_touching(pair: Pair) -> Stub {
    Stub::default()
        .object("pipe", 0.0)
        .object("wall", 0.5)
        .object("far-wall", 50.0)
        .pair("pipe", "wall", pair)
}

fn switched_off(mut rule: CompiledRule, switch: &str) -> CompiledRule {
    rule.parameters
        .insert(switch.into(), ParameterValue::Boolean { value: false });
    rule
}

/// Without a Hausdorff distance a touching pair could be a duplicate, so it
/// is not passed while duplicates are reported.
#[test]
fn an_unmeasured_hausdorff_distance_leaves_a_touching_pair_open() {
    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        walls_touching(overlapping(0.0).unmeasured_hausdorff()),
        &clash(&[("penetration_tolerance_metres", 0.01)]),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(outcome.not_evaluated_outcomes().len(), 1);

    // With duplicates switched off the question does not arise.
    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        walls_touching(overlapping(0.0).unmeasured_hausdorff()),
        &switched_off(
            clash(&[("penetration_tolerance_metres", 0.01)]),
            "report_duplicates",
        ),
    );
    assert!(outcome.findings().is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

#[test]
fn a_duplicate_within_tolerance_is_reported_as_a_duplicate() {
    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        walls_touching(overlapping(0.1).hausdorff(0.0, 0.002)),
        &clash(&[
            ("penetration_tolerance_metres", 0.01),
            ("duplicate_tolerance_metres", 0.005),
        ]),
    );
    assert!(one_message(&outcome).starts_with("duplicate of"));
}

/// A switched-off duplicate is not reported as the intersection it also is.
#[test]
fn a_switched_off_class_hides_its_pairs() {
    let rule = switched_off(
        clash(&[
            ("penetration_tolerance_metres", 0.01),
            ("duplicate_tolerance_metres", 0.005),
        ]),
        "report_duplicates",
    );
    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        walls_touching(overlapping(0.1).hausdorff(0.0, 0.002)),
        &rule,
    );
    assert!(outcome.findings().is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());

    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        walls_touching(overlapping(0.1)),
        &switched_off(
            clash(&[("penetration_tolerance_metres", 0.01)]),
            "report_intersections",
        ),
    );
    assert!(outcome.findings().is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

/// Straddling the duplicate tolerance: an intersection either way is still a
/// finding, but a pair that would pass unless it were a duplicate is open.
#[test]
fn a_straddling_duplicate_tolerance_decides_only_when_both_readings_agree() {
    let rule = clash(&[
        ("penetration_tolerance_metres", 0.01),
        ("duplicate_tolerance_metres", 0.005),
    ]);
    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        walls_touching(overlapping(0.1).hausdorff(0.001, 0.01)),
        &rule,
    );
    assert!(one_message(&outcome).starts_with("hard clash"));

    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        walls_touching(overlapping(0.0).hausdorff(0.001, 0.01)),
        &rule,
    );
    assert!(outcome.findings().is_empty());
    let [open] = outcome.not_evaluated_outcomes() else {
        panic!("one open pair expected");
    };
    assert!(open.message().contains("duplicate"), "{}", open.message());
}

fn axis_rule(horizontal: f64, vertical: f64) -> CompiledRule {
    clash(&[
        ("penetration_tolerance_metres", 0.0),
        ("horizontal_tolerance_metres", horizontal),
        ("vertical_tolerance_metres", vertical),
    ])
}

/// A 5 mm vertical overlap under a 10 mm vertical tolerance is no clash,
/// however wide it is in plan.
#[test]
fn an_intersection_counts_only_past_both_axis_tolerances() {
    let shallow = overlapping(0.005).extents([(2.0, 2.0), (0.5, 0.5), (0.005, 0.005)]);
    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        walls_touching(shallow),
        &axis_rule(0.01, 0.01),
    );
    assert!(outcome.findings().is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());

    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        walls_touching(shallow),
        &axis_rule(0.01, 0.001),
    );
    let message = one_message(&outcome);
    assert!(message.starts_with("hard clash"), "{message}");
    assert!(
        message.contains("reaching 0.5000 m in plan and 0.0050 m vertically"),
        "{message}"
    );

    // The narrower plan axis decides the horizontal tolerance.
    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        walls_touching(shallow),
        &axis_rule(0.6, 0.0),
    );
    assert!(outcome.findings().is_empty());
}

/// A pipe sunk into a wall sharing 0.2 m³ is a clash only past a volume
/// tolerance below that; an unmeasured or straddling volume is undecided.
#[test]
fn an_intersection_counts_only_past_the_volume_tolerance() {
    let volume_rule = |tolerance: f64| {
        clash(&[
            ("penetration_tolerance_metres", 0.0),
            ("volume_tolerance_cubic_metres", tolerance),
        ])
    };
    let sunk = overlapping(0.1).volume(0.2, 0.2);
    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        walls_touching(sunk),
        &volume_rule(0.1),
    );
    let message = one_message(&outcome);
    assert!(message.starts_with("hard clash"), "{message}");
    assert!(message.contains("sharing 0.200000 m³"), "{message}");

    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        walls_touching(sunk),
        &volume_rule(0.3),
    );
    assert!(outcome.findings().is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());

    for pair in [overlapping(0.1).volume(0.05, 0.15), overlapping(0.1)] {
        let outcome = run(
            &Clash,
            &pipes_and_walls(),
            walls_touching(pair),
            &volume_rule(0.1),
        );
        assert!(outcome.findings().is_empty());
        let [open] = outcome.not_evaluated_outcomes() else {
            panic!("one open pair expected");
        };
        assert!(
            open.message().contains("volume tolerance"),
            "{}",
            open.message()
        );
    }

    // No volume tolerance asks nothing of the volume.
    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        walls_touching(overlapping(0.1)),
        &volume_rule(0.0),
    );
    assert!(one_message(&outcome).starts_with("hard clash"));
}

#[test]
fn a_straddling_or_unmeasured_extent_is_not_evaluated() {
    for pair in [
        overlapping(0.05).extents([(0.1, 0.3), (0.5, 0.5), (1.0, 1.0)]),
        overlapping(0.05),
    ] {
        let outcome = run(
            &Clash,
            &pipes_and_walls(),
            walls_touching(pair),
            &axis_rule(0.2, 0.0),
        );
        assert!(outcome.findings().is_empty());
        assert_eq!(
            outcome.not_evaluated_outcomes()[0].reason(),
            &NotEvaluatedReason::IncompleteEvidence
        );
    }
    // A clearance shortfall holds whichever way the extent falls.
    let mut rule = axis_rule(0.2, 0.0);
    rule.parameters.insert(
        "clearance_metres".into(),
        ParameterValue::Number { value: 0.05 },
    );
    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        walls_touching(overlapping(0.05).extents([(0.1, 0.3), (0.5, 0.5), (1.0, 1.0)])),
        &rule,
    );
    assert!(one_message(&outcome).starts_with("clearance clash"));
}

#[test]
fn containment_has_its_own_switch() {
    let inside = Pair {
        separation: 0.2,
        penetration: Some(0.3),
        containment: Some(BodyContainment::SubjectInsideCounterpart),
        hausdorff: Some((0.2, 1.0)),
        extents: None,
        volume: None,
        certified: None,
    };
    let outcome = run(
        &Clash,
        &pipes_and_walls(),
        walls_touching(inside),
        &switched_off(
            clash(&[("penetration_tolerance_metres", 0.01)]),
            "report_containment",
        ),
    );
    assert!(outcome.findings().is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

/// The classes are data: a template whose first class is a clash of its
/// own (a penetration past a fixed depth) reports it before the built-in
/// classes are tried, over the same measured pairs.
#[test]
fn a_class_of_its_own_is_tried_first() {
    use axioval_engine::template::{Bound, Decision, PairClass, PairOrder, PairTest};
    let mut template = Clash.template().expect("clash runs as a template").clone();
    let Decision::Pairs(pairs) = &mut template.forms[0].decision else {
        panic!("clash judges pairs");
    };
    pairs.classes.insert(
        0,
        PairClass {
            name: "deep",
            holds: vec![PairTest::Compare {
                value: "penetration",
                order: PairOrder::Above,
                bound: Bound::Literal(0.05),
                zero: false,
            }],
            excused: None,
            reported: None,
            opens: false,
            fail: "deep clash with {counterpart}: {penetration:fixed4} m",
            undecided: "",
        },
    );
    let deep = axioval_rules::templates::Templated::new(template);
    let messages = |depth: f64| {
        let stub = Stub::default()
            .object("pipe", 0.0)
            .object("wall", 0.5)
            .object("far-wall", 50.0)
            .pair("pipe", "wall", overlapping(depth));
        let mut services = ServiceRegistry::new();
        services
            .register(ProximityServiceHandle::new(Arc::new(stub)))
            .unwrap();
        let project = pipes_and_walls();
        axioval_rules::register_builtins(CapabilityRegistry::new())
            .unwrap()
            .install_measured(&mut services, &project);
        deep.evaluate(
            &RuleContext {
                project: &project,
                services: &services,
            },
            &clash(&[("penetration_tolerance_metres", 0.01)]),
        )
        .findings()
        .iter()
        .map(|finding| finding.message.clone())
        .collect::<Vec<_>>()
    };
    assert_eq!(
        messages(0.1),
        ["deep clash with cad:model/wall: 0.1000 m".to_owned()]
    );
    assert_eq!(
        messages(0.03),
        [
            "hard clash with cad:model/wall: penetration 0.0300 m exceeds tolerance 0.0100 m"
                .to_owned()
        ]
    );
}

/// Generated pairs and declarations: every measurement the proximity
/// service may answer (or refuse) against every tolerance, switch,
/// clearance and grouping, the template held to the implementation it
/// replaced by `run`.
mod generated {
    use super::*;
    use proptest::prelude::*;

    fn interval(low: f64, wide: f64) -> (f64, f64) {
        (low, low + wide)
    }

    fn pair() -> impl Strategy<Value = Pair> {
        (
            prop_oneof![Just(0.0), 0.0..0.15f64],
            proptest::option::of(0.0..0.2f64),
            prop_oneof![
                6 => Just(None),
                1 => Just(Some(BodyContainment::SubjectInsideCounterpart)),
                1 => Just(Some(BodyContainment::CounterpartInsideSubject)),
            ],
            proptest::option::of((0.0..0.02f64, 0.0..0.05f64)),
            proptest::option::of([
                (0.0..0.3f64, 0.0..0.1f64),
                (0.0..0.3f64, 0.0..0.1f64),
                (0.0..0.3f64, 0.0..0.1f64),
            ]),
            proptest::option::of((0.0..0.002f64, 0.0..0.001f64)),
            proptest::option::of((0.0..0.1f64, 0.0..0.02f64)),
        )
            .prop_map(
                |(separation, penetration, containment, hausdorff, extents, volume, certified)| {
                    Pair {
                        separation,
                        penetration,
                        containment,
                        hausdorff: hausdorff.map(|(low, wide)| interval(separation + low, wide)),
                        extents: extents.map(|axes| axes.map(|(low, wide)| interval(low, wide))),
                        volume: volume.map(|(low, wide)| interval(low, wide)),
                        certified: certified.map(|(low, wide)| interval(low, wide)),
                    }
                },
            )
    }

    fn tolerance() -> impl Strategy<Value = Option<f64>> {
        proptest::option::of(prop_oneof![Just(0.0), Just(0.01), 0.0..0.1f64])
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(160))]

        #[test]
        fn generated_pairs_hold_parity(
            pairs in proptest::collection::vec(proptest::option::of(pair()), 4),
            tessellated in [any::<bool>(), any::<bool>()],
            penetration in prop_oneof![Just(0.0), Just(0.01), 0.0..0.1f64],
            clearance in proptest::option::of(prop_oneof![Just(0.02), 0.01..0.2f64]),
            duplicate in tolerance(),
            horizontal in tolerance(),
            vertical in tolerance(),
            volume in proptest::option::of(prop_oneof![Just(0.0), 0.0..0.002f64]),
            switches in [proptest::option::of(any::<bool>()), proptest::option::of(any::<bool>()), proptest::option::of(any::<bool>())],
            group in 0..5usize,
        ) {
            // Two pipes against two walls, every box overlapping.
            let mut stub = Stub::default();
            for (local, fidelity) in [("pipe", tessellated[0]), ("pipe-2", tessellated[1])] {
                stub = if fidelity { stub.tessellated(local, 0.0) } else { stub.object(local, 0.0) };
            }
            stub = stub.object("wall", 0.5).object("wall-2", 0.2);
            for ((subject, counterpart), pair) in
                [("pipe", "wall"), ("pipe", "wall-2"), ("pipe-2", "wall"), ("pipe-2", "wall-2")]
                    .into_iter()
                    .zip(pairs)
            {
                stub = match pair {
                    Some(pair) => stub.pair(subject, counterpart, pair),
                    None => stub.failing(subject, counterpart),
                };
            }
            let project = project(&[
                ("pipe", "pipe"),
                ("pipe-2", "pipe"),
                ("wall", "wall"),
                ("wall-2", "wall"),
            ]);
            let mut rule = clash(&[("penetration_tolerance_metres", penetration)]);
            for (name, value) in [
                ("clearance_metres", clearance),
                ("duplicate_tolerance_metres", duplicate),
                ("horizontal_tolerance_metres", horizontal),
                ("vertical_tolerance_metres", vertical),
                ("volume_tolerance_cubic_metres", volume),
            ] {
                if let Some(value) = value {
                    rule.parameters.insert(name.into(), ParameterValue::Number { value });
                }
            }
            for (name, value) in ["report_duplicates", "report_containment", "report_intersections"]
                .into_iter()
                .zip(switches)
            {
                if let Some(value) = value {
                    rule.parameters.insert(name.into(), ParameterValue::Boolean { value });
                }
            }
            let by = ["type_pair", "subject", "similar"];
            if let Some(by) = by.get(group) {
                rule.parameters.insert(
                    "group_by".into(),
                    ParameterValue::String { value: (*by).into() },
                );
                if *by == "similar" {
                    rule.parameters.insert(
                        "group_tolerance_metres".into(),
                        ParameterValue::Number { value: 0.05 },
                    );
                }
            }
            run(&Clash, &project, stub, &rule);
        }
    }
}

#[test]
fn switching_every_class_off_without_a_clearance_is_invalid() {
    let mut rule = clash(&[("penetration_tolerance_metres", 0.01)]);
    for switch in [
        "report_duplicates",
        "report_containment",
        "report_intersections",
    ] {
        rule = switched_off(rule, switch);
    }
    let outcome = run(&Clash, &pipes_and_walls(), Stub::default(), &rule);
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::InvalidDeclaration
    );
}
