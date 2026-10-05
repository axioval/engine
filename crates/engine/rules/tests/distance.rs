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
    Bounds3, CapabilityEvaluation, ElevationInterval, GeometryFidelity, NotEvaluatedReason,
    ObjectBounds, ProjectedDistanceEvidence, ProximityError, ProximityEvidence, ProximityRequest,
    ProximityService, ProximityServiceHandle, RuleCapability, VerticalExtent, VerticalExtentError,
    VerticalExtentService, VerticalExtentServiceHandle,
};
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
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
            Some(0.0),
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
                "Vertical { footprint_offset_metres: 0.0, direction: Either, surfaces: Extents }",
                (2.0, 2.0),
            )
            .distance(
                "pipe",
                "mid",
                "Vertical { footprint_offset_metres: 0.0, direction: Either, surfaces: Extents }",
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
            "Vertical { footprint_offset_metres: 0.5, direction: Either, surfaces: Extents }",
            (2.0, 2.0),
        );
    let mut parameters = at_least(1, 3.0);
    parameters.push(("projection", string("vertical")));
    parameters.push(("footprint_offset_metres", number(0.5)));
    let outcome = run(model(), stub, parameters);
    assert!(outcome.findings().is_empty() && outcome.not_evaluated_outcomes().is_empty());
}

/// A sprinkler at most 0.5 m below the ceiling: only counterparts above
/// count, and the broad phase never proposes the floor below (the stub would
/// panic on it). Nothing above within 2 m: a counterpart above too close is a
/// finding naming the direction.
#[test]
fn a_vertical_direction_counts_only_its_side() {
    let stub = || {
        Stub::default()
            .at("pipe", 0.0, 0.0)
            .at("near", 0.5, 1.5)
            .at("mid", 0.2, -1.4)
            .at("far", 20.0, 0.0)
            .distance(
                "pipe",
                "near",
                "Vertical { footprint_offset_metres: 0.0, direction: Above, surfaces: Extents }",
                (0.5, 0.5),
            )
            .distance(
                "pipe",
                "mid",
                "Vertical { footprint_offset_metres: 0.0, direction: Below, surfaces: Extents }",
                (0.4, 0.4),
            )
    };
    let declare = |direction: &str, parameters: Vec<(&'static str, ParameterValue)>| {
        let mut parameters = parameters;
        parameters.push(("projection", string("vertical")));
        parameters.push(("vertical_direction", string(direction)));
        parameters
    };
    // At most 0.5 m below the ceiling: the wall above is 0.5 m up.
    let outcome = run(
        model(),
        stub(),
        declare("above", vec![("maximum_metres", number(0.5))]),
    );
    assert!(outcome.findings().is_empty() && outcome.not_evaluated_outcomes().is_empty());
    // At least 0.5 m clear below: the one under the pipe is 0.4 m down.
    let outcome = run(
        model(),
        stub(),
        declare("below", vec![("minimum_metres", number(0.5))]),
    );
    let finding = only_finding(&outcome);
    assert_eq!(finding.related, vec![id("mid")]);
    assert!(
        finding.message.contains("vertical distance 0.4000 m below"),
        "{}",
        finding.message
    );
    // Nothing above within 2 m.
    let outcome = run(
        model(),
        stub(),
        declare(
            "above",
            vec![
                ("mode", string("none_closer_than")),
                ("minimum_metres", number(2.0)),
            ],
        ),
    );
    let finding = only_finding(&outcome);
    assert_eq!(finding.related, vec![id("near")]);
    assert!(
        finding.message.contains("vertical distance 0.5000 m above"),
        "{}",
        finding.message
    );
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

/// Two zones hold the pipe: a fire zone with `mid` and a zone of another
/// kind with `near`. Only fire zones scope the counterparts.
#[test]
fn counterparts_share_only_containers_of_the_declared_kind() {
    let zoned = || {
        model()
            .object("fire", "firezone")
            .object("lighting", "zone")
            .edge("groups", "fire", "pipe")
            .edge("groups", "fire", "mid")
            .edge("groups", "lighting", "pipe")
            .edge("groups", "lighting", "near")
    };
    let apart = |kinds: bool| {
        let mut parameters = vec![
            ("mode", string("none_closer_than")),
            ("minimum_metres", number(0.6)),
            ("path", strings(&["groups:backward"])),
        ];
        if kinds {
            parameters.push(("container_selector", selector(kind("firezone"))));
        }
        parameters
    };
    // Sharing any zone, `near` is 0.5 m away.
    let outcome = run(zoned(), walls(), apart(false));
    assert_eq!(only_finding(&outcome).related, vec![id("near")]);
    // Sharing only a zone of another kind, the two are not paired.
    let outcome = run(zoned(), walls(), apart(true));
    assert!(outcome.findings().is_empty(), "{:?}", outcome.findings());
    assert!(outcome.not_evaluated_outcomes().is_empty());
    // A container the selector cannot decide leaves the pair undecided.
    let outcome = run(
        zoned().unreadable("lighting"),
        walls(),
        vec![
            ("mode", string("none_closer_than")),
            ("minimum_metres", number(0.6)),
            ("path", strings(&["groups:backward"])),
            (
                "container_selector",
                selector(Selector::AnyOf {
                    operands: vec![kind("firezone"), unreadable_property()],
                }),
            ),
        ],
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        reasons(&outcome),
        vec![("pipe".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

/// A selector the model cannot decide for objects `unreadable` marks.
fn unreadable_property() -> Selector {
    Selector::Property {
        property_set: Some("Pset".into()),
        property: "Kind".into(),
        operator: ComparisonOperator::Exists,
        value: None,
        case_sensitive: true,
        trim: false,
        quantifier: None,
        precision: None,
    }
}

impl VerticalExtentService for Stub {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        let (_, z, _) = self
            .boxes
            .get(&object.local_id)
            .copied()
            .flatten()
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?;
        VerticalExtent::try_new(
            object.clone(),
            ElevationInterval::exact(z)?,
            ElevationInterval::exact(z + 1.0)?,
            Evidence::exact(source(), format!("extent:{}", object.local_id)),
        )
    }
}

fn run_with_heights(stub: Stub, parameters: Vec<(&str, ParameterValue)>) -> CapabilityEvaluation {
    let stub = Arc::new(stub);
    model().evaluate_with(&Distance, &check(parameters), |services| {
        services
            .register(ProximityServiceHandle::new(stub.clone()))
            .unwrap();
        services
            .register(VerticalExtentServiceHandle::new(stub))
            .unwrap();
    })
}

/// `near` stands beside the pipe, `far` beside it one storey up.
#[test]
fn a_counterpart_on_another_storey_is_ignored_under_overlapping() {
    let storeys = || {
        Stub::default()
            .at("pipe", 0.0, 0.0)
            .at("near", 1.5, 0.0)
            .at("far", 1.5, 3.0)
            .distance("pipe", "near", "Horizontal", (0.5, 0.5))
            .distance("pipe", "far", "Horizontal", (0.4, 0.4))
    };
    let apart = |overlap: Option<(&'static str, f64)>| {
        let mut parameters = vec![
            ("mode", string("none_closer_than")),
            ("minimum_metres", number(0.6)),
            ("projection", string("horizontal")),
        ];
        if let Some((overlap, offset)) = overlap {
            parameters.push(("elevation_overlap", string(overlap)));
            parameters.push(("elevation_offset_metres", number(offset)));
        }
        parameters
    };
    let outcome = run_with_heights(storeys(), apart(None));
    assert_eq!(only_finding(&outcome).related, vec![id("far"), id("near")]);
    let outcome = run_with_heights(storeys(), apart(Some(("overlapping", 0.0))));
    assert_eq!(only_finding(&outcome).related, vec![id("near")]);
    // Within a 2.5 m band around the pipe's heights the upper one counts.
    let outcome = run_with_heights(storeys(), apart(Some(("overlapping", 2.5))));
    assert_eq!(only_finding(&outcome).related, vec![id("far"), id("near")]);
    // Without the vertical-extent service the rule is not evaluated.
    let outcome = run(model(), storeys(), apart(Some(("overlapping", 0.0))));
    assert!(outcome.findings().is_empty());
    assert_eq!(
        reasons(&outcome),
        vec![("-".to_owned(), NotEvaluatedReason::MissingService)]
    );
}

#[test]
fn a_distance_between_surfaces_is_requested_and_graded() {
    let stub = Stub::default()
        .at("pipe", 0.0, 0.0)
        .at("near", 0.0, 1.3)
        .at("mid", 10.0, 0.0)
        .at("far", 20.0, 0.0)
        .distance(
            "pipe",
            "near",
            "Vertical { footprint_offset_metres: 0.0, direction: Above, surfaces: Between { \
             subject: Top, counterpart: Nearest } }",
            (0.875, 0.875),
        );
    let outcome = run(
        model(),
        stub,
        vec![
            ("maximum_metres", number(0.5)),
            ("projection", string("vertical")),
            ("vertical_direction", string("above")),
            ("subject_surface", string("top")),
            ("counterpart_surface", string("nearest")),
        ],
    );
    let finding = only_finding(&outcome);
    assert_eq!(
        finding.message,
        "nearest counterpart test:model/near is at vertical distance from its top to the nearest surface \
         0.8750 m above, farther than the allowed 0.5000 m"
    );
    let deviation = outcome.deviation(0).expect("graded");
    assert!(
        (deviation.lower() - 0.75).abs() < 1e-9 && (deviation.upper() - 0.75).abs() < 1e-9,
        "{deviation:?}"
    );
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
        vec![
            ("maximum_metres", number(1.0)),
            ("vertical_direction", string("above")),
        ],
        vec![
            ("maximum_metres", number(1.0)),
            ("projection", string("horizontal")),
            ("vertical_direction", string("below")),
        ],
        vec![
            ("maximum_metres", number(1.0)),
            ("projection", string("vertical")),
            ("vertical_direction", string("sideways")),
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

#[test]
fn invalid_surface_height_and_container_declarations_are_refused() {
    let declarations: Vec<Vec<(&str, ParameterValue)>> = vec![
        vec![
            ("maximum_metres", number(1.0)),
            ("projection", string("horizontal")),
            ("subject_surface", string("top")),
            ("counterpart_surface", string("top")),
        ],
        vec![
            ("maximum_metres", number(1.0)),
            ("projection", string("vertical")),
            ("subject_surface", string("top")),
        ],
        vec![
            ("maximum_metres", number(1.0)),
            ("projection", string("vertical")),
            ("subject_surface", string("middle")),
            ("counterpart_surface", string("top")),
        ],
        vec![
            ("maximum_metres", number(1.0)),
            ("projection", string("vertical")),
            ("subject_surface", string("top")),
            ("counterpart_surface", string("nearest")),
            ("footprint_offset_metres", number(0.5)),
        ],
        vec![
            ("maximum_metres", number(1.0)),
            ("projection", string("vertical")),
            ("elevation_overlap", string("overlapping")),
        ],
        vec![
            ("maximum_metres", number(1.0)),
            ("projection", string("horizontal")),
            ("elevation_overlap", string("always")),
        ],
        vec![
            ("maximum_metres", number(1.0)),
            ("projection", string("horizontal")),
            ("elevation_offset_metres", number(0.5)),
        ],
        vec![
            ("maximum_metres", number(1.0)),
            ("container_selector", selector(kind("zone"))),
        ],
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
        "vertical_direction",
        "relationship",
        "path",
        "subject_surface",
        "counterpart_surface",
        "elevation_overlap",
        "elevation_offset_metres",
        "container_selector",
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

#[test]
fn a_computed_minimum_judges_as_the_literal_it_computes() {
    let mut literal = at_least(2, 1.0);
    literal.push(("minimum_metres", number(0.6)));
    let mut computed = at_least(2, 1.0);
    computed.push((
        "minimum_metres",
        common::expression(serde_json::json!({"kind": "divide",
            "left": {"kind": "literal", "value": {"type": "number", "value": 1.2}},
            "right": {"kind": "literal", "value": {"type": "integer", "value": 2}}})),
    ));
    let expected = run(model(), walls(), literal);
    let found = run(model(), walls(), computed);
    assert_eq!(found.findings(), expected.findings());
    assert!(
        found.findings()[0]
            .message
            .contains("between 0.6000 and 1.0000 m")
    );
}

/// A verdict three ways: holds, fails, or cannot be decided.
#[derive(Debug, PartialEq)]
enum Tri {
    Holds,
    Fails,
    Open,
}

/// The capability's verdict about the pipe.
fn capability_verdict(outcome: &CapabilityEvaluation) -> Tri {
    if outcome
        .findings()
        .iter()
        .any(|finding| common::subject(finding) == "pipe")
    {
        Tri::Fails
    } else if reasons(outcome).iter().any(|(object, _)| object == "pipe") {
        Tri::Open
    } else {
        Tri::Holds
    }
}

/// The pipe's measured value `name`, judged at least `bound`, or at most
/// with a negative `bound` (Kleene: an interval straddling it is open);
/// `absent` is the verdict when there is no value.
fn measured_verdict(model: &Model, stub: Stub, name: &str, bound: f64, absent: &Tri) -> Tri {
    use axioval_engine::{CapabilityRegistry, PropertyResolution, ServiceRegistry, measured_value};
    use axioval_ir::PropertyValue;
    let project = model.project();
    let mut services = ServiceRegistry::new();
    services
        .register(ProximityServiceHandle::new(Arc::new(stub)))
        .unwrap();
    axioval_rules::register_builtins(CapabilityRegistry::new())
        .unwrap()
        .install_measured(&mut services, &project);
    let (lower, upper) = match measured_value(&services, &project, &id("pipe"), name) {
        Ok(PropertyResolution::Present(resolved)) => match resolved.property().value() {
            PropertyValue::Integer(value) => {
                #[allow(clippy::cast_precision_loss)]
                let value = *value as f64;
                (value, value)
            }
            PropertyValue::Quantity { value, .. } | PropertyValue::Decimal(value) => {
                (*value, *value)
            }
            PropertyValue::Measured { lower, upper, .. } => (*lower, *upper),
            other => panic!("{name}: {other:?}"),
        },
        Ok(PropertyResolution::Absent(_)) => {
            return match absent {
                Tri::Holds => Tri::Holds,
                Tri::Fails => Tri::Fails,
                Tri::Open => Tri::Open,
            };
        }
        Err(_) => return Tri::Open,
    };
    let (lower, upper, bound) = if bound < 0.0 {
        (-upper, -lower, bound)
    } else {
        (lower, upper, bound)
    };
    if lower >= bound {
        Tri::Holds
    } else if upper < bound {
        Tri::Fails
    } else {
        Tri::Open
    }
}

/// `nearest` with a maximum: the nearest counterpart's distance at most
/// the maximum, none within it failing.
#[test]
fn the_nearest_distance_reaches_the_nearest_modes_verdicts() {
    for (maximum, expected) in [(1.0, Tri::Holds), (0.4, Tri::Fails)] {
        let verdict = capability_verdict(&run(
            model(),
            walls(),
            vec![("maximum_metres", number(maximum))],
        ));
        assert_eq!(verdict, expected);
        let name = format!("distance;to=wall;within={maximum}");
        assert_eq!(
            measured_verdict(&model(), walls(), &name, -maximum, &Tri::Fails),
            expected,
            "at most {maximum}"
        );
    }
}

/// Every mode's fixtures, rewritten as a measured value and a comparison,
/// reach the capability's verdict: `at_least` as `count_within` at least
/// the count, `none_closer_than` as the nearest `distance` at least the
/// minimum (none within it holds).
#[test]
#[allow(clippy::too_many_lines, clippy::items_after_statements)]
fn measured_distances_reach_the_capabilitys_verdicts_in_every_mode() {
    let undecided = || {
        Stub::default()
            .at("pipe", 0.0, 0.0)
            .at("near", 1.5, 0.0)
            .curved("mid", 1.8, 0.0)
            .at("far", 4.0, 0.0)
            .distance("pipe", "near", "Minimum3d", (0.5, 0.5))
            .distance("pipe", "mid", "Minimum3d", (0.95, 1.05))
            .distance("pipe", "far", "Minimum3d", (3.0, 3.0))
    };
    let unmeasured = || {
        Stub::default()
            .at("pipe", 0.0, 0.0)
            .at("near", 1.5, 0.0)
            .without_geometry("mid")
            .at("far", 4.0, 0.0)
            .distance("pipe", "near", "Minimum3d", (0.5, 0.5))
    };
    let straddling = |near: (f64, f64)| {
        Stub::default()
            .at("pipe", 0.0, 0.0)
            .at("near", 1.5, 0.0)
            .curved("mid", 1.5, 0.0)
            .at("far", 4.0, 0.0)
            .distance("pipe", "near", "Minimum3d", near)
            .distance("pipe", "mid", "Minimum3d", (0.58, 0.62))
    };
    let planar = || {
        Stub::default()
            .at("pipe", 0.0, 0.0)
            .at("near", 1.5, 10.0)
            .at("mid", 9.0, 10.0)
            .at("far", 20.0, 0.0)
            .distance("pipe", "near", "Horizontal", (0.5, 0.5))
    };
    let none_closer = |minimum: f64| {
        vec![
            ("mode", string("none_closer_than")),
            ("minimum_metres", number(minimum)),
        ]
    };
    let ranged = {
        let mut parameters = at_least(2, 1.0);
        parameters.push(("minimum_metres", number(0.6)));
        parameters
    };
    let horizontal = {
        let mut parameters = at_least(1, 1.0);
        parameters.push(("projection", string("horizontal")));
        parameters
    };
    type Case = (
        &'static str,
        Box<dyn Fn() -> Stub>,
        Vec<(&'static str, ParameterValue)>,
        String,
        f64,
    );
    let count = |radius: f64| format!("count_within;to=wall;radius={radius}");
    let nearest = |within: f64| format!("distance;to=wall;within={within}");
    let cases: Vec<Case> = vec![
        (
            "2 within 1 m",
            Box::new(walls),
            at_least(2, 1.0),
            count(1.0),
            2.0,
        ),
        (
            "3 within 1 m",
            Box::new(walls),
            at_least(3, 1.0),
            count(1.0),
            3.0,
        ),
        (
            "2 within 0.6 to 1 m",
            Box::new(walls),
            ranged,
            format!("{};from=0.6", count(1.0)),
            2.0,
        ),
        (
            "1 of undecided",
            Box::new(undecided),
            at_least(1, 1.0),
            count(1.0),
            1.0,
        ),
        (
            "2 of undecided",
            Box::new(undecided),
            at_least(2, 1.0),
            count(1.0),
            2.0,
        ),
        (
            "3 of undecided",
            Box::new(undecided),
            at_least(3, 1.0),
            count(1.0),
            3.0,
        ),
        (
            "2 of unmeasured",
            Box::new(unmeasured),
            at_least(2, 1.0),
            count(1.0),
            2.0,
        ),
        (
            "3 of unmeasured",
            Box::new(unmeasured),
            at_least(3, 1.0),
            count(1.0),
            3.0,
        ),
        (
            "1 in plan",
            Box::new(planar),
            horizontal,
            format!("{};projection=horizontal", count(1.0)),
            1.0,
        ),
        (
            "none within 1 m",
            Box::new(walls),
            none_closer(1.0),
            nearest(1.0),
            1.0,
        ),
        (
            "none within 0.4 m",
            Box::new(walls),
            none_closer(0.4),
            nearest(0.4),
            0.4,
        ),
        (
            "none within a straddle",
            Box::new(move || straddling((0.7, 0.7))),
            none_closer(0.6),
            nearest(0.6),
            0.6,
        ),
        (
            "certainly one within",
            Box::new(move || straddling((0.5, 0.5))),
            none_closer(0.6),
            nearest(0.6),
            0.6,
        ),
    ];
    for (case, stub, parameters, name, bound) in cases {
        let expected = capability_verdict(&run(model(), stub(), parameters));
        let measured = measured_verdict(&model(), stub(), &name, bound, &Tri::Holds);
        assert_eq!(measured, expected, "{case}: `{name}` at least {bound}");
    }
}

/// The capability under `parameters` and the expression rule
/// `requirement` over the pipes, both measuring through `stub`, compared
/// by the parity harness.
fn expression_parity(
    stub: &dyn Fn() -> Stub,
    parameters: Vec<(&str, ParameterValue)>,
    requirement: &serde_json::Value,
) -> axioval_rules::parity::ParityEvidence {
    let expected = run(model(), stub(), parameters);
    let rule = rule(
        "axioval:capability.expression",
        kind("pipe"),
        vec![("requirement", common::expression(requirement.clone()))],
    );
    let outcome =
        model().evaluate_measured(&axioval_rules::ExpressionRequirement, &rule, |services| {
            services
                .register(ProximityServiceHandle::new(Arc::new(stub())))
                .unwrap();
        });
    axioval_rules::parity::compare_evaluations((CAPABILITY, &expected), ("expression", &outcome))
}

/// Every mode's fixtures as expression rules over `count_within` and
/// `distance`, held to the parity harness: `at_least` as the count within
/// at least the count, `none_closer_than` as the nearest distance, where
/// one is within, at least the minimum, `nearest` as the nearest distance
/// at most the maximum. The capability also leaves an unmeasurable
/// counterpart open, which an expression over the subjects never reports.
#[test]
#[allow(clippy::too_many_lines, clippy::type_complexity)]
fn distance_expressions_hold_to_the_parity_harness_in_every_mode() {
    use axioval_rules::parity::{Difference, Outcome};
    use serde_json::{Value, json};
    let measured = |name: &str| json!({"kind": "property", "propertySet": "axioval:measured", "property": name});
    let at_least_count = |radius: &str, count: f64| {
        json!({"kind": "compare", "operator": "greaterThanOrEquals",
            "left": measured(&format!("count_within;to=wall;radius={radius}")),
            "right": {"kind": "literal", "value": {"type": "number", "value": count}}})
    };
    let m = |value: f64| json!({"kind": "literal", "value": {"type": "quantity", "value": value, "unit": "m"}});
    let none_closer = |minimum: f64| {
        let nearest = measured(&format!("distance;to=wall;within={minimum}"));
        json!({"kind": "implies", "antecedent": {"kind": "isDefined", "operand": nearest.clone()},
            "consequent": {"kind": "compare", "operator": "greaterThanOrEquals",
                "left": nearest, "right": m(minimum)}})
    };
    let nearest_within = |maximum: f64| {
        json!({"kind": "compare", "operator": "lessThanOrEquals",
            "left": measured(&format!("distance;to=wall;within={maximum}")), "right": m(maximum)})
    };
    let undecided = || {
        Stub::default()
            .at("pipe", 0.0, 0.0)
            .at("near", 1.5, 0.0)
            .curved("mid", 1.8, 0.0)
            .at("far", 4.0, 0.0)
            .distance("pipe", "near", "Minimum3d", (0.5, 0.5))
            .distance("pipe", "mid", "Minimum3d", (0.95, 1.05))
            .distance("pipe", "far", "Minimum3d", (3.0, 3.0))
    };
    let unmeasured = || {
        Stub::default()
            .at("pipe", 0.0, 0.0)
            .at("near", 1.5, 0.0)
            .without_geometry("mid")
            .at("far", 4.0, 0.0)
            .distance("pipe", "near", "Minimum3d", (0.5, 0.5))
    };
    let straddling = |near: (f64, f64)| {
        move || {
            Stub::default()
                .at("pipe", 0.0, 0.0)
                .at("near", 1.5, 0.0)
                .curved("mid", 1.5, 0.0)
                .at("far", 4.0, 0.0)
                .distance("pipe", "near", "Minimum3d", near)
                .distance("pipe", "mid", "Minimum3d", (0.58, 0.62))
        }
    };
    let planar = || {
        Stub::default()
            .at("pipe", 0.0, 0.0)
            .at("near", 1.5, 10.0)
            .at("mid", 9.0, 10.0)
            .at("far", 20.0, 0.0)
            .distance("pipe", "near", "Horizontal", (0.5, 0.5))
    };
    let mode = |mode: &str, bound: (&'static str, f64)| {
        vec![("mode", string(mode)), (bound.0, number(bound.1))]
    };
    let ranged = {
        let mut parameters = at_least(2, 1.0);
        parameters.push(("minimum_metres", number(0.6)));
        parameters
    };
    let horizontal = {
        let mut parameters = at_least(1, 1.0);
        parameters.push(("projection", string("horizontal")));
        parameters
    };
    let in_plan = json!({"kind": "compare", "operator": "greaterThanOrEquals",
        "left": measured("count_within;to=wall;radius=1;projection=horizontal"),
        "right": {"kind": "literal", "value": {"type": "number", "value": 1.0}}});
    let straddled_near = straddling((0.7, 0.7));
    let surely_near = straddling((0.5, 0.5));
    let cases: Vec<(&str, &dyn Fn() -> Stub, Vec<(&str, ParameterValue)>, Value)> = vec![
        (
            "2 within 1 m",
            &walls,
            at_least(2, 1.0),
            at_least_count("1", 2.0),
        ),
        (
            "3 within 1 m",
            &walls,
            at_least(3, 1.0),
            at_least_count("1", 3.0),
        ),
        (
            "2 within 0.6 to 1 m",
            &walls,
            ranged,
            at_least_count("1;from=0.6", 2.0),
        ),
        (
            "1 of undecided",
            &undecided,
            at_least(1, 1.0),
            at_least_count("1", 1.0),
        ),
        (
            "2 of undecided",
            &undecided,
            at_least(2, 1.0),
            at_least_count("1", 2.0),
        ),
        (
            "3 of undecided",
            &undecided,
            at_least(3, 1.0),
            at_least_count("1", 3.0),
        ),
        ("1 in plan", &planar, horizontal, in_plan),
        (
            "none within 1 m",
            &walls,
            mode("none_closer_than", ("minimum_metres", 1.0)),
            none_closer(1.0),
        ),
        (
            "none within 0.4 m",
            &walls,
            mode("none_closer_than", ("minimum_metres", 0.4)),
            none_closer(0.4),
        ),
        (
            "none within a straddle",
            &straddled_near,
            mode("none_closer_than", ("minimum_metres", 0.6)),
            none_closer(0.6),
        ),
        (
            "certainly one within",
            &surely_near,
            mode("none_closer_than", ("minimum_metres", 0.6)),
            none_closer(0.6),
        ),
        (
            "nearest within 1 m",
            &walls,
            vec![("maximum_metres", number(1.0))],
            nearest_within(1.0),
        ),
        (
            "nearest within 0.4 m",
            &walls,
            vec![("maximum_metres", number(0.4))],
            nearest_within(0.4),
        ),
    ];
    for (case, stub, parameters, requirement) in cases {
        let parity = expression_parity(stub, parameters, &requirement);
        assert!(parity.holds(), "{case}:\n{}", parity.diff());
    }
    // An unmeasurable counterpart: the pipe alike, the counterpart itself
    // left open by the capability alone.
    for (count, open) in [(2, true), (3, false)] {
        let parity = expression_parity(
            &unmeasured,
            at_least(count, 1.0),
            &at_least_count("1", f64::from(u8::try_from(count).unwrap())),
        );
        assert_eq!(
            parity.differences,
            vec![Difference {
                scope: id("mid").into(),
                capability: Some(Outcome::NotEvaluated {
                    reason: NotEvaluatedReason::IncompleteEvidence
                }),
                expression: None,
                details: vec![],
            }],
            "{count} of unmeasured"
        );
        assert_eq!(parity.open, 1 + usize::from(open));
    }
}
