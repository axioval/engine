//! Accessible routes judged over real Axiolid walkability.
//!
//! Room `a` (x 0..4) is the start. Room `b` (x 4.2..8) lies east of it
//! through `door`, 0.9 m wide, and room `d` (x -4..-0.2) west of it through
//! `narrow`, also 0.9 m wide; both walls are 0.2 m thick, split into a south
//! piece, a north piece and a lintel above 2.1 m. Room `c` lies above `a` on
//! the next level, reached only by `stair`. Clear widths come from a small
//! in-memory source: 0.85 m for `door`, 0.75 m for `narrow`.
#![cfg(feature = "axiolid")]
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval::axiolid::{AxiolidGeometry, AxiolidWalkabilityService};
use axioval::engine::{
    CapabilityEvaluation, CompiledRule, CompletePropertyAbsenceEvidence, DoorLeaf, DoorLeaves,
    DoorLeavesError, HingeSide, LeafMotion, LeafPosition, MetricDirection, ObjectFrame,
    ObjectFrameError, ObjectFrameService, ObjectFrameServiceHandle, PropertyRequest,
    PropertyResolution, PropertyResolutionError, PropertyResolutionService,
    PropertyResolutionServiceHandle, ResolvedProperty, RuleCapability, RuleContext,
    ServiceRegistry, SourceSnapshot, SwingSector, WalkabilityServiceHandle,
};
use axioval::ir::contract::{ParameterValue, Selector, Severity};
use axioval::ir::{
    Evidence, NotEvaluatedReason, Object, ObjectId, Project, Property, PropertyValue,
    QuantityDimension, RuleId, SourceId,
};
use axioval::rules::AccessibleRoute;

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

/// A closed, outward-oriented box.
fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    TriMesh::new(
        vec![
            Point3::new(x0, y0, z0),
            Point3::new(x1, y0, z0),
            Point3::new(x1, y1, z0),
            Point3::new(x0, y1, z0),
            Point3::new(x0, y0, z1),
            Point3::new(x1, y0, z1),
            Point3::new(x1, y1, z1),
            Point3::new(x0, y1, z1),
        ],
        vec![
            0, 2, 1, 0, 3, 2, // floor, facing down
            4, 5, 6, 4, 6, 7, // ceiling, facing up
            0, 1, 5, 0, 5, 4, // sides
            1, 2, 6, 1, 6, 5, //
            2, 3, 7, 2, 7, 6, //
            3, 0, 4, 3, 4, 7,
        ],
    )
}

const PSET: &str = "Accessibility";
const CLEAR_WIDTH: &str = "ClearWidth";

struct Scene {
    objects: Vec<Object>,
    geometry: AxiolidGeometry,
    widths: BTreeMap<ObjectId, f64>,
    leaves: BTreeMap<ObjectId, DoorLeaves>,
}

impl Scene {
    fn new() -> Self {
        let mut scene = Self::empty();
        for (local, kind, min, max) in [
            ("a", "lobby", [0.0, 0.0, 0.0], [4.0, 4.0, 3.0]),
            ("b", "room", [4.2, 0.0, 0.0], [8.0, 4.0, 3.0]),
            ("c", "room", [0.0, 0.0, 3.3], [4.0, 4.0, 6.3]),
            ("d", "room", [-4.0, 0.0, 0.0], [-0.2, 4.0, 3.0]),
        ] {
            scene = scene.body(local, kind, cuboid(min, max));
        }
        for (x0, x1, door) in [(4.0, 4.2, "door"), (-0.2, 0.0, "narrow")] {
            scene = scene
                .body(
                    &format!("{door}-wall-s"),
                    "wall",
                    cuboid([x0, 0.0, 0.0], [x1, 1.0, 3.0]),
                )
                .body(
                    &format!("{door}-wall-n"),
                    "wall",
                    cuboid([x0, 1.9, 0.0], [x1, 4.0, 3.0]),
                )
                .body(
                    &format!("{door}-lintel"),
                    "wall",
                    cuboid([x0, 1.0, 2.1], [x1, 1.9, 3.0]),
                )
                .body(
                    door,
                    "door",
                    cuboid([x0 + 0.05, 1.0, 0.0], [x1 - 0.05, 1.9, 2.1]),
                );
        }
        // Far enough from `b` and `d` (more than 1 m in plan) to join only
        // `a` and `c`.
        scene
            .body("stair", "stair", cuboid([1.5, 0.5, 0.0], [2.5, 3.0, 3.3]))
            .width("door", 0.85)
            .width("narrow", 0.75)
    }

    fn empty() -> Self {
        Self {
            objects: Vec::new(),
            geometry: AxiolidGeometry::new(),
            widths: BTreeMap::new(),
            leaves: BTreeMap::new(),
        }
    }

    /// Lobby `a` (x 0..4) opens through `door` onto corridor `e`, 1.2 m
    /// wide (y 1..2.2) and 6 m long, which opens through `far` onto room
    /// `f` (x 10.2..14). Cupboard `hatch`, in the corridor's north wall at
    /// x 6.5..7.4, has a 0.9 m leaf that swings south across the corridor.
    fn corridor() -> Self {
        let mut scene = Self::empty();
        for (local, kind, min, max) in [
            ("a", "lobby", [0.0, 0.0, 0.0], [4.0, 4.0, 3.0]),
            ("e", "corridor", [4.2, 1.0, 0.0], [10.0, 2.2, 3.0]),
            ("f", "room", [10.2, 0.0, 0.0], [14.0, 4.0, 3.0]),
        ] {
            scene = scene.body(local, kind, cuboid(min, max));
        }
        for (x0, x1, door) in [(4.0, 4.2, "door"), (10.0, 10.2, "far")] {
            scene = scene
                .body(
                    &format!("{door}-wall-s"),
                    "wall",
                    cuboid([x0, 0.0, 0.0], [x1, 1.15, 3.0]),
                )
                .body(
                    &format!("{door}-wall-n"),
                    "wall",
                    cuboid([x0, 2.05, 0.0], [x1, 4.0, 3.0]),
                )
                .body(
                    &format!("{door}-lintel"),
                    "wall",
                    cuboid([x0, 1.15, 2.1], [x1, 2.05, 3.0]),
                )
                .body(
                    door,
                    "door",
                    cuboid([x0 + 0.05, 1.15, 0.0], [x1 - 0.05, 2.05, 2.1]),
                )
                .width(door, 0.85);
        }
        scene = scene.body(
            "hatch",
            "cupboard",
            cuboid([6.5, 2.2, 0.0], [7.4, 2.3, 2.0]),
        );
        let sector = SwingSector::try_new(
            [6.5, 2.2, 0.0],
            0.9,
            direction([1.0, 0.0, 0.0]),
            direction([0.0, -1.0, 0.0]),
            false,
        )
        .unwrap();
        let leaf = DoorLeaf::try_new(
            LeafPosition::NotDefined,
            LeafMotion::Swing,
            [6.5, 2.2, 0.0],
            direction([1.0, 0.0, 0.0]),
            direction([0.0, -1.0, 0.0]),
            direction([0.0, 0.0, 1.0]),
            0.9,
            Some(0.04),
            Some(HingeSide::Right),
            Some(sector),
        )
        .unwrap();
        scene.leaves.insert(
            id("hatch"),
            DoorLeaves::try_new(
                id("hatch"),
                "SINGLE_SWING_RIGHT",
                0.9,
                None,
                vec![leaf],
                Evidence::exact(source(), "leaves:hatch"),
            )
            .unwrap(),
        );
        scene
    }

    fn body(mut self, local: &str, kind: &str, mesh: TriMesh) -> Self {
        self.objects.push(Object::new(id(local), kind));
        self.geometry = self.geometry.with_mesh(id(local), mesh);
        self
    }

    fn width(mut self, local: &str, metres: f64) -> Self {
        self.widths.insert(id(local), metres);
        self
    }

    /// The source states no clear width at all.
    fn unstated(mut self) -> Self {
        self.widths.clear();
        self
    }

    fn check(self, parameters: &[(&str, ParameterValue)]) -> CapabilityEvaluation {
        let mut bound: BTreeMap<String, ParameterValue> = BTreeMap::from([
            (
                "route_selector".to_owned(),
                selector(Selector::AnyOf {
                    operands: vec![kind("lobby"), kind("room"), kind("corridor")],
                }),
            ),
            ("start_selector".to_owned(), selector(kind("lobby"))),
            ("portal_selector".to_owned(), selector(kind("door"))),
            ("stair_selector".to_owned(), selector(kind("stair"))),
            ("obstacle_selector".to_owned(), selector(kind("wall"))),
            ("width_metres".to_owned(), number(0.8)),
            ("door_width_metres".to_owned(), number(0.8)),
            (
                "clear_width_property".to_owned(),
                ParameterValue::PropertyReference {
                    property: CLEAR_WIDTH.into(),
                    property_set: Some(PSET.into()),
                },
            ),
        ]);
        for (name, value) in parameters {
            bound.insert((*name).to_owned(), value.clone());
        }
        let rule = CompiledRule {
            id: RuleId::new("accessible-route").unwrap(),
            capability: "axioval:capability.accessible-route".into(),
            severity: Severity::Error,
            selector: kind("room"),
            parameters: bound,
        };
        let project = Project::new(self.objects.clone()).unwrap();
        let walkability = AxiolidWalkabilityService::new(self.geometry.clone(), source());
        let mut services = ServiceRegistry::new();
        services
            .register(WalkabilityServiceHandle::new(Arc::new(walkability)))
            .unwrap();
        services
            .register(ObjectFrameServiceHandle::new(Arc::new(Leaves {
                snapshots: vec![SourceSnapshot::try_new(source(), "r1", "sha256:1").unwrap()],
                leaves: self.leaves.clone(),
            })))
            .unwrap();
        services
            .register(PropertyResolutionServiceHandle::new(Arc::new(self)))
            .unwrap();
        AccessibleRoute.evaluate(
            &RuleContext {
                project: &project,
                services: &services,
            },
            &rule,
        )
    }
}

impl PropertyResolutionService for Scene {
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        assert_eq!(request.property_set(), Some(PSET));
        assert_eq!(request.property(), CLEAR_WIDTH);
        let object = request.object_id();
        match self.widths.get(object) {
            Some(metres) => {
                let value = PropertyValue::Quantity {
                    value: *metres,
                    dimension: QuantityDimension::Length,
                };
                let property = Property::new(PSET, CLEAR_WIDTH, value)
                    .unwrap()
                    .with_evidence(Evidence::exact(source(), format!("clear-width:{object}")));
                Ok(PropertyResolution::Present(ResolvedProperty::try_new(
                    request.clone(),
                    property,
                )?))
            }
            None => Ok(PropertyResolution::Absent(
                CompletePropertyAbsenceEvidence::try_new(
                    request.clone(),
                    Evidence::exact(source(), format!("no-clear-width:{object}")),
                )?,
            )),
        }
    }
}

/// Door leaves as a source would state them.
struct Leaves {
    snapshots: Vec<SourceSnapshot>,
    leaves: BTreeMap<ObjectId, DoorLeaves>,
}

impl ObjectFrameService for Leaves {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn object_frame(&self, object: &ObjectId) -> Result<ObjectFrame, ObjectFrameError> {
        Err(ObjectFrameError::NotPlaced(object.clone()))
    }

    fn leaves(&self, door: &ObjectId) -> Result<DoorLeaves, DoorLeavesError> {
        self.leaves
            .get(door)
            .cloned()
            .ok_or_else(|| DoorLeavesError::NotADoor(door.clone()))
    }
}

fn direction(vector: [f64; 3]) -> MetricDirection {
    MetricDirection::try_new(vector).unwrap()
}

fn kind(kind: &str) -> Selector {
    Selector::EntityType {
        object_type: kind.into(),
        include_subtypes: false,
    }
}

fn selector(value: Selector) -> ParameterValue {
    ParameterValue::Selector {
        value: Box::new(value),
    }
}

fn number(value: f64) -> ParameterValue {
    ParameterValue::Number { value }
}

fn flag(value: bool) -> ParameterValue {
    ParameterValue::Boolean { value }
}

/// `(destination, related, message)` of every finding, sorted.
fn findings(outcome: &CapabilityEvaluation) -> Vec<(String, Vec<String>, String)> {
    let mut found: Vec<_> = outcome
        .findings()
        .iter()
        .map(|finding| {
            let axioval::ir::Scope::Object(object) = &finding.scope else {
                panic!("object finding expected: {finding:?}");
            };
            (
                object.local_id.clone(),
                finding
                    .related
                    .iter()
                    .map(|related| related.local_id.clone())
                    .collect(),
                finding.message.clone(),
            )
        })
        .collect();
    found.sort();
    found
}

/// `(object, reason, message)` of every not-evaluated outcome, sorted.
fn unevaluated(outcome: &CapabilityEvaluation) -> Vec<(String, NotEvaluatedReason, String)> {
    let mut found: Vec<_> = outcome
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| {
            let axioval::ir::Scope::Object(object) = outcome.scope() else {
                panic!("object outcome expected: {outcome:?}");
            };
            (
                object.local_id.clone(),
                outcome.reason().clone(),
                outcome.message().to_owned(),
            )
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

#[test]
fn a_continuous_route_passes_and_a_narrow_door_and_a_stair_block() {
    let outcome = Scene::new().check(&[]);
    // `b` is reached through `door`, stated 0.85 m clear: it passes.
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
    let found = findings(&outcome);
    assert_eq!(found.len(), 2, "{found:#?}");
    // `c` is reached by the stair alone.
    assert_eq!(found[0].0, "c");
    assert_eq!(found[0].1, vec!["stair".to_owned()]);
    assert!(
        found[0]
            .2
            .starts_with("cad:model/c is connected to the starts by stairs only"),
        "{found:#?}"
    );
    // `d` is behind `narrow`, stated 0.75 m clear.
    assert_eq!(found[1].0, "d");
    assert_eq!(found[1].1, vec!["narrow".to_owned()]);
    assert!(
        found[1]
            .2
            .contains("cad:model/narrow states a clear width of 0.75 m"),
        "{found:#?}"
    );
    let evidence = &outcome.findings()[1].evidence;
    assert!(
        evidence
            .iter()
            .any(|evidence| evidence.locator == "clear-width:cad:model/narrow"),
        "{evidence:#?}"
    );
}

#[test]
fn a_stair_allowed_by_the_rule_leaves_the_upper_room_undecided() {
    // Climbs are not measured, so a route by the stair is never proven.
    let outcome = Scene::new().check(&[("forbid_stairs", flag(false))]);
    let found = findings(&outcome);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].0, "d");
    let open = unevaluated(&outcome);
    assert_eq!(open.len(), 1, "{open:#?}");
    assert_eq!(open[0].0, "c");
    assert_eq!(open[0].1, NotEvaluatedReason::IncompleteEvidence);
}

#[test]
fn a_door_narrower_than_the_door_minimum_blocks_even_without_a_stated_width() {
    // Without stated widths nothing passes: a door's leaf and lining are
    // unknown. The geometry still shows both doors narrower than 1 m.
    let outcome = Scene::new()
        .unstated()
        .check(&[("door_width_metres", number(1.0))]);
    let found = findings(&outcome);
    assert_eq!(found.len(), 3, "{found:#?}");
    assert_eq!(found[0].0, "b");
    assert_eq!(found[0].1, vec!["door".to_owned()]);
    assert!(
        found[0].2.contains(
            "cad:model/door is at most 0.9002 m wide in the geometry, less than the 1 m required"
        ),
        "{found:#?}"
    );
}

#[test]
fn without_stated_widths_a_door_that_could_pass_is_undecided() {
    let outcome = Scene::new().unstated().check(&[]);
    let open = unevaluated(&outcome);
    assert_eq!(
        open.iter()
            .map(|(object, ..)| object.as_str())
            .collect::<Vec<_>>(),
        ["b", "d"],
        "{open:#?}"
    );
    assert!(
        open[0].2.contains("cad:model/door states no clear width"),
        "{open:#?}"
    );
    // The stair still blocks `c`.
    let found = findings(&outcome);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].0, "c");
}

#[test]
fn a_corridor_narrowed_by_an_open_leaf_blocks_the_room_beyond() {
    // Without the swing, the 1.2 m corridor carries the 0.8 m body to `f`.
    let outcome = Scene::corridor().check(&[]);
    assert!(findings(&outcome).is_empty(), "{outcome:#?}");
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
    // The hatch's leaf leaves 0.3 m of it: `f` is cut off.
    let outcome = Scene::corridor().check(&[("subtract_door_swings", selector(kind("cupboard")))]);
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
    let found = findings(&outcome);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].0, "f");
    assert!(
        found[0]
            .2
            .starts_with("no accessible route reaches cad:model/f for a body 0.8 m wide"),
        "{found:#?}"
    );
    // The block is the corridor's floor under the swing, located there.
    assert!(
        outcome.findings()[0]
            .evidence
            .iter()
            .any(|evidence| evidence
                .locator
                .starts_with("axiolid:walkability:stretch:cad:model/e:obstructed:at=")),
        "{outcome:#?}"
    );
}
