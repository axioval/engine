//! Containment capability contract tests.
//!
//! ADR 0004: the service measures shared volumes and face distances, the
//! capability decides. The stub answers only the pairs and distances a test
//! declares and panics on any other pair, which also proves the broad phase
//! kept far-apart pairs away from the narrow phase.
#![allow(missing_docs)]

use std::{collections::BTreeMap, sync::Arc};

use axioval_engine::{
    Bounds3, CapabilityEvaluation, CompiledRule, FaceClass, FaceDistanceError,
    FaceDistanceEvidence, FaceDistanceRequest, GeometryFidelity, IntersectionVolume,
    NotEvaluatedReason, ObjectBounds, ProximityError, ProximityEvidence, ProximityRequest,
    ProximityService, ProximityServiceHandle, RuleCapability, RuleContext, ServiceRegistry,
    SignedDistanceInterval, VolumeInterval,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity as RuleSeverity, TableRow};
use axioval_ir::{Evidence, Object, ObjectId, Project, RuleId, Scope, SourceId};
use axioval_rules::Containment;

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}
fn oid(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

/// A measured pair: separation and volumes, `(lower, upper)` each.
#[derive(Clone, Copy)]
struct Pair {
    separation: f64,
    shared: (f64, f64),
    first: (f64, f64),
    second: (f64, f64),
}

/// A pair sharing `shared` of a 1 m³ first body and a 10 m³ second.
fn sharing(lower: f64, upper: f64) -> Pair {
    Pair {
        separation: 0.0,
        shared: (lower, upper),
        first: (1.0, 1.0),
        second: (10.0, 10.0),
    }
}

type Extent = Option<([f64; 3], [f64; 3])>;
type FaceKey = (String, String, FaceClass);
type FaceAnswer = Result<(f64, f64), FaceDistanceError>;

#[derive(Default)]
struct Stub {
    boxes: BTreeMap<String, Extent>,
    pairs: BTreeMap<(String, String), Result<Pair, ProximityError>>,
    faces: BTreeMap<FaceKey, FaceAnswer>,
}

impl Stub {
    fn object(mut self, local: &str, min: [f64; 3], max: [f64; 3]) -> Self {
        self.boxes.insert(local.into(), Some((min, max)));
        self
    }
    fn unmeasured(mut self, local: &str) -> Self {
        self.boxes.insert(local.into(), None);
        self
    }
    fn pair(mut self, a: &str, b: &str, pair: Pair) -> Self {
        self.pairs.insert((a.into(), b.into()), Ok(pair));
        self
    }
    fn face(mut self, body: &str, host: &str, class: FaceClass, lower: f64, upper: f64) -> Self {
        self.faces
            .insert((body.into(), host.into(), class), Ok((lower, upper)));
        self
    }
    fn face_error(
        mut self,
        body: &str,
        host: &str,
        class: FaceClass,
        error: FaceDistanceError,
    ) -> Self {
        self.faces
            .insert((body.into(), host.into(), class), Err(error));
        self
    }
}

fn evidence(locator: String) -> Evidence {
    Evidence {
        source: source(),
        locator,
        exact: true,
    }
}

impl ProximityService for Stub {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        let (min, max) = self
            .boxes
            .get(&object.local_id)
            .copied()
            .flatten()
            .ok_or(ProximityError::Unavailable)?;
        ObjectBounds::try_new(
            object.clone(),
            Bounds3::try_new(min, max)?,
            GeometryFidelity::Exact,
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
        let (pair, swapped) = match self.pairs.get(&(a.clone(), b.clone())) {
            Some(pair) => (pair, false),
            None => (
                self.pairs
                    .get(&(b.clone(), a.clone()))
                    .unwrap_or_else(|| panic!("broad phase should have pruned {a}/{b}")),
                true,
            ),
        };
        let pair = (*pair)?;
        let (subject, counterpart) = if swapped {
            (pair.second, pair.first)
        } else {
            (pair.first, pair.second)
        };
        let interval = |(lower, upper): (f64, f64)| VolumeInterval::try_new(lower, upper).unwrap();
        ProximityEvidence::try_new(
            request.clone(),
            pair.separation,
            Some(if pair.shared.0 > 0.0 { 0.1 } else { 0.0 }),
            Some(0.0),
            None,
            GeometryFidelity::Exact,
            evidence(format!("proximity:{a}:{b}")),
        )?
        .with_intersection_volume(IntersectionVolume::try_new(
            interval(pair.shared),
            interval(subject),
            interval(counterpart),
        )?)
    }

    fn measure_face_distance(
        &self,
        request: &FaceDistanceRequest,
    ) -> Result<FaceDistanceEvidence, FaceDistanceError> {
        let key = (
            request.body().local_id.clone(),
            request.host().local_id.clone(),
            request.faces(),
        );
        let (lower, upper) = (*self
            .faces
            .get(&key)
            .unwrap_or_else(|| panic!("unexpected face distance {key:?}")))?;
        FaceDistanceEvidence::try_new(
            request.clone(),
            SignedDistanceInterval::try_new(lower, upper).unwrap(),
            GeometryFidelity::Exact,
            evidence(format!("faces:{}:{}", key.0, key.1)),
        )
    }
}

fn kind(object_type: &str) -> Selector {
    Selector::EntityType {
        object_type: object_type.into(),
        include_subtypes: false,
    }
}

fn number(value: f64) -> ParameterValue {
    ParameterValue::Number { value }
}

fn band(faces: &str, side: Option<&str>, minimum: Option<f64>, maximum: Option<f64>) -> TableRow {
    let mut row = TableRow::new();
    row.insert(
        "faces".into(),
        ParameterValue::String {
            value: faces.into(),
        },
    );
    if let Some(side) = side {
        row.insert("side".into(), ParameterValue::String { value: side.into() });
    }
    if let Some(minimum) = minimum {
        row.insert("minimum_metres".into(), number(minimum));
    }
    if let Some(maximum) = maximum {
        row.insert("maximum_metres".into(), number(maximum));
    }
    row
}

fn rule(parameters: Vec<(&str, ParameterValue)>) -> CompiledRule {
    let mut bound = BTreeMap::from([
        (
            "counterparts".to_string(),
            ParameterValue::Selector {
                value: Box::new(kind("wall")),
            },
        ),
        ("minimum_volume_ratio".to_string(), number(0.9)),
    ]);
    for (name, value) in parameters {
        bound.insert(name.to_string(), value);
    }
    CompiledRule {
        id: RuleId::new("check").unwrap(),
        capability: "axioval:capability.containment".into(),
        severity: RuleSeverity::Error,
        selector: kind("column"),
        parameters: bound,
    }
}

fn cover(rows: Vec<TableRow>) -> (&'static str, ParameterValue) {
    ("cover", ParameterValue::Table { value: rows })
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

fn run(project: &Project, stub: Stub, rule: &CompiledRule) -> CapabilityEvaluation {
    let mut services = ServiceRegistry::new();
    services
        .register(ProximityServiceHandle::new(Arc::new(stub)))
        .unwrap();
    Containment.evaluate(
        &RuleContext {
            project,
            services: &services,
        },
        rule,
    )
}

fn messages(outcome: &CapabilityEvaluation) -> Vec<String> {
    outcome
        .findings()
        .iter()
        .map(|finding| finding.message.clone())
        .collect()
}

fn open(outcome: &CapabilityEvaluation) -> Vec<String> {
    outcome
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| outcome.message().to_owned())
        .collect()
}

const WALL: ([f64; 3], [f64; 3]) = ([0.0, 0.0, 0.0], [4.0, 0.3, 3.0]);
const COLUMN: ([f64; 3], [f64; 3]) = ([1.0, 0.02, 0.4], [1.2, 0.22, 2.5]);

/// A column in a wall, 0.02 m from the side faces, 0.5 m under the top and
/// 0.4 m over the bottom; a far wall the broad phase keeps away.
fn column_in_wall() -> Stub {
    Stub::default()
        .object("wall", WALL.0, WALL.1)
        .object("column", COLUMN.0, COLUMN.1)
        .object("far-wall", [50.0, 0.0, 0.0], [54.0, 0.3, 3.0])
        .pair("column", "wall", sharing(1.0, 1.0))
        .face("column", "wall", FaceClass::Side, 0.02, 0.02)
        .face("column", "wall", FaceClass::Top, 0.5, 0.5)
        .face("column", "wall", FaceClass::Bottom, 0.4, 0.4)
        .face("column", "wall", FaceClass::Any, 0.02, 0.02)
}

fn column_and_walls() -> Project {
    project(&[("column", "column"), ("wall", "wall"), ("far-wall", "wall")])
}

/// Too little side cover is found, naming the wall; the top cover passes.
#[test]
fn a_column_inside_a_wall_with_too_little_side_cover_is_found() {
    let outcome = run(
        &column_and_walls(),
        column_in_wall(),
        &rule(vec![cover(vec![
            band("side", None, Some(0.05), None),
            band("top", Some("inside"), Some(0.3), Some(1.0)),
        ])]),
    );
    assert!(open(&outcome).is_empty(), "{:?}", open(&outcome));
    let [finding] = outcome.findings() else {
        panic!("one finding expected: {:?}", messages(&outcome));
    };
    assert_eq!(finding.scope, Scope::Object(oid("column")));
    assert_eq!(finding.related, vec![oid("wall")]);
    assert_eq!(
        finding.message,
        "side cover to cad:model/wall is 0.0200 m, below the minimum 0.0500 m"
    );
    assert_eq!(finding.evidence.len(), 2);
}

/// Each face class is judged against its own band: one test per class, each
/// failing only its own.
#[test]
fn each_face_class_is_judged_on_its_own() {
    for (class, minimum, holds) in [
        ("top", 0.6, false),
        ("top", 0.5, true),
        ("side", 0.03, false),
        ("side", 0.02, true),
        ("bottom", 0.45, false),
        ("bottom", 0.4, true),
        ("any", 0.021, false),
        ("any", 0.01, true),
    ] {
        let outcome = run(
            &column_and_walls(),
            column_in_wall(),
            &rule(vec![cover(vec![band(class, None, Some(minimum), None)])]),
        );
        assert!(open(&outcome).is_empty());
        let found = messages(&outcome);
        assert_eq!(found.is_empty(), holds, "{class} {minimum}: {found:?}");
        if !holds {
            assert!(found[0].starts_with(&format!("{class} cover")), "{found:?}");
        }
    }
}

/// A maximum is a bound too: a column set too deep is found.
#[test]
fn a_cover_above_its_maximum_is_found() {
    let outcome = run(
        &column_and_walls(),
        column_in_wall(),
        &rule(vec![cover(vec![band("top", None, None, Some(0.4))])]),
    );
    assert_eq!(
        messages(&outcome),
        vec!["top cover to cad:model/wall is 0.5000 m, above the maximum 0.4000 m"]
    );
}

/// An outside band bounds how far the body reaches past the faces: the
/// negated signed distance.
#[test]
fn an_outside_band_bounds_the_protrusion() {
    let protruding = || {
        Stub::default()
            .object("wall", WALL.0, WALL.1)
            .object("column", [1.0, 0.05, 0.5], [1.2, 0.25, 3.4])
            .pair("column", "wall", sharing(0.95, 0.95))
            .face("column", "wall", FaceClass::Top, -0.44, -0.4)
    };
    let project = project(&[("column", "column"), ("wall", "wall")]);
    let rule_with = |minimum, maximum| {
        rule(vec![cover(vec![band(
            "top",
            Some("outside"),
            minimum,
            maximum,
        )])])
    };
    let outcome = run(&project, protruding(), &rule_with(Some(0.3), Some(0.5)));
    assert!(outcome.findings().is_empty() && open(&outcome).is_empty());
    let outcome = run(&project, protruding(), &rule_with(Some(0.5), None));
    assert_eq!(
        messages(&outcome),
        vec![
            "reach past the top faces of cad:model/wall is 0.4000 to 0.4400 m, below the minimum 0.5000 m"
        ]
    );
    // A bound inside the interval is undecided.
    let outcome = run(&project, protruding(), &rule_with(None, Some(0.42)));
    assert!(outcome.findings().is_empty());
    assert!(
        open(&outcome)[0].contains("cannot be decided"),
        "{:?}",
        open(&outcome)
    );
}

#[test]
fn a_straddling_cover_or_unmeasured_face_distance_is_not_evaluated() {
    let stub = column_in_wall().face("column", "wall", FaceClass::Side, 0.01, 0.04);
    let outcome = run(
        &column_and_walls(),
        stub,
        &rule(vec![cover(vec![band("side", None, Some(0.02), None)])]),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::IncompleteEvidence
    );

    let stub = column_in_wall().face_error(
        "column",
        "wall",
        FaceClass::Side,
        FaceDistanceError::Unsupported,
    );
    let outcome = run(
        &column_and_walls(),
        stub,
        &rule(vec![cover(vec![band("side", None, Some(0.02), None)])]),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::BackendUnavailable
    );
}

/// A shared volume whose ratio straddles the declared one leaves the column
/// undecided: neither in the wall nor an orphan, and its cover unchecked.
#[test]
fn an_undecided_containment_is_not_evaluated() {
    let stub = Stub::default()
        .object("wall", WALL.0, WALL.1)
        .object("column", COLUMN.0, COLUMN.1)
        .pair("column", "wall", sharing(0.85, 0.95));
    let project = project(&[("column", "column"), ("wall", "wall")]);
    let outcome = run(
        &project,
        stub,
        &rule(vec![
            cover(vec![band("side", None, Some(0.02), None)]),
            ("report_orphans", ParameterValue::Boolean { value: true }),
        ]),
    );
    assert!(outcome.findings().is_empty());
    let messages = open(&outcome);
    assert!(
        messages
            .iter()
            .any(|message| message.contains("0.8500 to 0.9500")),
        "{messages:?}"
    );
}

/// A column in no wall is an orphan; with a wall unmeasured it may not be.
#[test]
fn an_orphan_is_found_only_when_every_wall_was_measured() {
    let orphans = ("report_orphans", ParameterValue::Boolean { value: true });
    let free = || {
        Stub::default()
            .object("wall", WALL.0, WALL.1)
            .object("column", [1.0, 0.2, 0.0], [1.2, 0.5, 3.0])
            .pair("column", "wall", sharing(0.1, 0.1))
    };
    let project_with = |walls: &[&str]| {
        let mut objects = vec![("column", "column")];
        objects.extend(walls.iter().map(|wall| (*wall, "wall")));
        project(&objects)
    };
    let outcome = run(
        &project_with(&["wall"]),
        free(),
        &rule(vec![orphans.clone()]),
    );
    assert_eq!(
        messages(&outcome),
        vec!["lies in no outer element: none shares 0.9000 of the smaller body's volume"]
    );

    let outcome = run(
        &project_with(&["wall", "lost"]),
        free().unmeasured("lost"),
        &rule(vec![orphans]),
    );
    assert!(outcome.findings().is_empty());
    assert!(
        outcome
            .not_evaluated_outcomes()
            .iter()
            .any(|open| open.object_id() == Some(&oid("column"))),
        "{:?}",
        open(&outcome)
    );
}

/// Two columns in a wall allowed one: the wall is found. A third column
/// undecided leaves a minimum of three open.
#[test]
fn counts_per_outer_element_are_judged_when_undecided_ones_cannot_change_them() {
    let stub = || {
        Stub::default()
            .object("wall", WALL.0, WALL.1)
            .object("a", [0.5, 0.05, 0.5], [0.7, 0.25, 2.5])
            .object("b", [2.5, 0.05, 0.5], [2.7, 0.25, 2.5])
            .object("c", [3.5, 0.05, 0.5], [3.7, 0.25, 2.5])
            .pair("a", "wall", sharing(1.0, 1.0))
            .pair("b", "wall", sharing(1.0, 1.0))
            .pair("c", "wall", sharing(0.8, 0.95))
    };
    let project = project(&[
        ("a", "column"),
        ("b", "column"),
        ("c", "column"),
        ("wall", "wall"),
    ]);
    let count = |name: &str, value: i64| (name.to_owned(), ParameterValue::Integer { value });
    let with = |counts: Vec<(String, ParameterValue)>| {
        let mut rule = rule(Vec::new());
        rule.parameters.extend(counts);
        rule
    };
    let outcome = run(&project, stub(), &with(vec![count("maximum_count", 1)]));
    let [finding] = outcome.findings() else {
        panic!("one finding expected: {:?}", messages(&outcome));
    };
    assert_eq!(finding.scope, Scope::Object(oid("wall")));
    assert_eq!(finding.related, vec![oid("a"), oid("b")]);
    assert_eq!(
        finding.message,
        "holds 2 inner elements, more than the maximum 1"
    );

    let outcome = run(&project, stub(), &with(vec![count("minimum_count", 3)]));
    assert!(outcome.findings().is_empty());
    assert!(
        open(&outcome)
            .iter()
            .any(|message| message.contains("between 2 and 3")),
        "{:?}",
        open(&outcome)
    );

    let outcome = run(&project, stub(), &with(vec![count("minimum_count", 4)]));
    assert_eq!(
        messages(&outcome),
        vec!["holds 2 inner elements, fewer than the minimum 4"]
    );

    let outcome = run(
        &project,
        stub(),
        &with(vec![count("minimum_count", 2), count("maximum_count", 3)]),
    );
    assert!(outcome.findings().is_empty());
    assert!(
        outcome
            .not_evaluated_outcomes()
            .iter()
            .all(|open| open.object_id() != Some(&oid("wall"))),
        "{:?}",
        open(&outcome)
    );
}

/// A column at the junction of two walls lies half in each: in neither
/// alone, but in both combined when combining is declared.
#[test]
fn adjacent_outer_elements_can_be_combined() {
    let stub = || {
        Stub::default()
            .object("east", [0.0, 0.0, 0.0], [4.0, 0.3, 3.0])
            .object("north", [4.0, 0.0, 0.0], [4.3, 4.0, 3.0])
            .object("column", [3.8, 0.05, 0.5], [4.2, 0.25, 2.5])
            .pair("column", "east", sharing(0.5, 0.5))
            .pair("column", "north", sharing(0.5, 0.5))
            .pair(
                "east",
                "north",
                Pair {
                    separation: 0.0,
                    shared: (0.0, 0.0),
                    first: (3.6, 3.6),
                    second: (3.6, 3.6),
                },
            )
    };
    let project = project(&[("column", "column"), ("east", "wall"), ("north", "wall")]);
    let orphans = ("report_orphans", ParameterValue::Boolean { value: true });
    let outcome = run(&project, stub(), &rule(vec![orphans.clone()]));
    assert_eq!(messages(&outcome).len(), 1, "{:?}", messages(&outcome));

    let combine = ("combine_adjacent", ParameterValue::Boolean { value: true });
    let outcome = run(
        &project,
        stub(),
        &rule(vec![orphans.clone(), combine.clone()]),
    );
    assert!(outcome.findings().is_empty(), "{:?}", messages(&outcome));
    assert!(open(&outcome).is_empty(), "{:?}", open(&outcome));

    // Each wall holds the column it shares volume with.
    let mut counted = rule(vec![combine.clone()]);
    counted
        .parameters
        .insert("maximum_count".into(), ParameterValue::Integer { value: 0 });
    let outcome = run(&project, stub(), &counted);
    assert_eq!(outcome.findings().len(), 2, "{:?}", messages(&outcome));

    // The combination's faces are not one body's: cover is not checked.
    let outcome = run(
        &project,
        stub(),
        &rule(vec![
            combine,
            cover(vec![band("side", None, Some(0.02), None)]),
        ]),
    );
    assert!(outcome.findings().is_empty());
    assert!(
        open(&outcome)[0].contains("combination"),
        "{:?}",
        open(&outcome)
    );
}

#[test]
fn unusable_declarations_are_refused() {
    let project = column_and_walls();
    for parameters in [
        vec![
            ("minimum_volume_ratio", number(0.0)),
            cover(vec![band("top", None, Some(0.1), None)]),
        ],
        vec![
            ("minimum_volume_ratio", number(1.5)),
            cover(vec![band("top", None, Some(0.1), None)]),
        ],
        vec![cover(vec![band("front", None, Some(0.1), None)])],
        vec![cover(vec![band("top", Some("above"), Some(0.1), None)])],
        vec![cover(vec![band("top", None, None, None)])],
        vec![cover(vec![band("top", None, Some(0.2), Some(0.1))])],
        vec![cover(vec![
            band("top", None, Some(0.1), None),
            band("top", Some("inside"), None, Some(0.5)),
        ])],
        vec![("minimum_count", ParameterValue::Integer { value: -1 })],
        vec![],
    ] {
        let outcome = run(&project, Stub::default(), &rule(parameters));
        assert!(outcome.findings().is_empty());
        let [refused] = outcome.not_evaluated_outcomes() else {
            panic!("the column is refused: {:?}", open(&outcome));
        };
        assert_eq!(refused.reason(), &NotEvaluatedReason::InvalidDeclaration);
    }
}
