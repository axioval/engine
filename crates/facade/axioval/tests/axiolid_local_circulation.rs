//! Local circulation within rooms, measured on real meshes.
//!
//! A room 6 m by 4 m has a door in its south wall at x 0.5..1.5. Which
//! space the door opens onto and which space a component stands in are
//! derived from the geometry too (`axioval:derived.adjacent-space` and
//! `axioval:derived.contained-in-space`), so the whole check runs on the
//! meshes.
#![cfg(feature = "axiolid")]
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval::axiolid::{
    AxiolidDerivedRelationshipService, AxiolidFreeSpaceService, AxiolidGeometry,
    AxiolidProximityService,
};
use axioval::engine::{
    CapabilityEvaluation, CompiledRule, CompleteRelationshipSelection,
    DerivedRelationshipServiceHandle, DoorLeaf, DoorLeaves, DoorLeavesError,
    FreeSpaceServiceHandle, HingeSide, LeafMotion, LeafPosition, MetricDirection, ObjectFrame,
    ObjectFrameError, ObjectFrameService, ObjectFrameServiceHandle, ProximityServiceHandle,
    RelationshipQuery, RelationshipSelectionError, RelationshipSelectionRequest,
    RelationshipSelectionService, RelationshipSelectionServiceHandle, RuleCapability, RuleContext,
    ServiceRegistry, SourceSnapshot, SwingSector,
};
use axioval::ir::contract::{ParameterValue, Selector, Severity};
use axioval::ir::{Evidence, NotEvaluatedReason, Object, ObjectId, Project, RuleId, SourceId};
use axioval::rules::LocalCirculation;

const ADJACENT: &str = "axioval:derived.adjacent-space";
const CONTAINED: &str = "axioval:derived.contained-in-space";

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
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7,
            6, 3, 0, 4, 3, 4, 7,
        ],
    )
}

struct Scene {
    objects: Vec<Object>,
    geometry: AxiolidGeometry,
    leaves: BTreeMap<ObjectId, DoorLeaves>,
    /// Further spaces, and pairs a stated `merges` relationship joins.
    spaces: Vec<&'static str>,
    merges: Vec<(ObjectId, ObjectId)>,
}

impl Scene {
    /// The room and its door.
    fn room() -> Self {
        Self {
            objects: Vec::new(),
            geometry: AxiolidGeometry::new(),
            leaves: BTreeMap::new(),
            spaces: Vec::new(),
            merges: Vec::new(),
        }
        .body("room", "room", cuboid([0.0, 0.0, 0.0], [6.0, 4.0, 3.0]))
        .body("door", "door", cuboid([0.5, -0.2, 0.0], [1.5, 0.0, 2.1]))
    }

    /// A partition from the south wall at x 3.0..3.1 up to `y`, and a WC
    /// behind it in the south-east corner.
    fn partitioned(y: f64) -> Self {
        Self::room()
            .body("partition", "wall", cuboid([3.0, 0.0, 0.0], [3.1, y, 3.0]))
            .body("wc", "wc", cuboid([5.3, 0.2, 0.0], [5.9, 0.9, 0.8]))
    }

    fn body(mut self, local: &str, kind: &str, mesh: TriMesh) -> Self {
        self.objects.push(Object::new(id(local), kind));
        self.geometry = self.geometry.with_mesh(id(local), mesh);
        self
    }

    /// `local`'s 0.9 m leaf, hinged at `hinge` on the floor, closed along
    /// `closed` and opening towards `open`.
    fn swinging(mut self, local: &str, hinge: [f64; 3], closed: [f64; 3], open: [f64; 3]) -> Self {
        self.leaves
            .insert(id(local), hinged(local, hinge, closed, open));
        self
    }

    fn check(self, parameters: &[(&str, ParameterValue)]) -> CapabilityEvaluation {
        let mut bound: BTreeMap<String, ParameterValue> = BTreeMap::from([
            ("component_selector".to_owned(), selector(kind("wc"))),
            ("space_path".to_owned(), strings(&[CONTAINED])),
            ("access_path".to_owned(), strings(&[ADJACENT])),
            ("door_selector".to_owned(), selector(kind("door"))),
            ("space_selector".to_owned(), selector(kind("room"))),
            ("width_metres".to_owned(), number(0.9)),
            ("clear_height_metres".to_owned(), number(2.0)),
        ]);
        for (name, value) in parameters {
            bound.insert((*name).to_owned(), value.clone());
        }
        let rule = CompiledRule {
            id: RuleId::new("local-circulation").unwrap(),
            capability: "axioval:capability.local-circulation".into(),
            severity: Severity::Error,
            selector: kind("room"),
            parameters: bound,
        };
        let project = Project::new(self.objects.clone()).unwrap();
        let mut derived = AxiolidDerivedRelationshipService::new(self.geometry.clone())
            .with_space(id("room"))
            .with_opening(id("door"));
        for space in &self.spaces {
            derived = derived.with_space(id(space));
        }
        let mut services = ServiceRegistry::new();
        services
            .register(RelationshipSelectionServiceHandle::new(Arc::new(Derived(
                DerivedRelationshipServiceHandle::new(Arc::new(derived)),
                self.merges.clone(),
            ))))
            .unwrap();
        services
            .register(ProximityServiceHandle::new(Arc::new(
                AxiolidProximityService::new(self.geometry.clone()),
            )))
            .unwrap();
        services
            .register(FreeSpaceServiceHandle::new(Arc::new(
                AxiolidFreeSpaceService::new(self.geometry, source()),
            )))
            .unwrap();
        services
            .register(ObjectFrameServiceHandle::new(Arc::new(Leaves {
                snapshots: vec![SourceSnapshot::try_new(source(), "r1", "sha256:1").unwrap()],
                leaves: self.leaves,
            })))
            .unwrap();
        LocalCirculation.evaluate(
            &RuleContext {
                project: &project,
                services: &services,
            },
            &rule,
        )
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

/// One hinged leaf 0.9 m wide.
fn hinged(local: &str, hinge: [f64; 3], closed: [f64; 3], open: [f64; 3]) -> DoorLeaves {
    let side = if closed[0] * open[1] - closed[1] * open[0] > 0.0 {
        HingeSide::Left
    } else {
        HingeSide::Right
    };
    let sector =
        SwingSector::try_new(hinge, 0.9, direction(closed), direction(open), false).unwrap();
    let leaf = DoorLeaf::try_new(
        LeafPosition::NotDefined,
        LeafMotion::Swing,
        hinge,
        direction(closed),
        direction(open),
        direction([0.0, 0.0, 1.0]),
        0.9,
        Some(0.04),
        Some(side),
        Some(sector),
    )
    .unwrap();
    DoorLeaves::try_new(
        id(local),
        "SINGLE_SWING",
        0.9,
        None,
        vec![leaf],
        Evidence::exact(source(), format!("leaves:{local}")),
    )
    .unwrap()
}

/// Every relationship derived from the geometry, and a stated `merges`
/// relationship between the given pairs, either way.
struct Derived(DerivedRelationshipServiceHandle, Vec<(ObjectId, ObjectId)>);

impl RelationshipSelectionService for Derived {
    fn select(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        let RelationshipQuery::Related { relationship, .. } = request.query() else {
            return self.0.select(request);
        };
        if relationship.as_str() != "merges" {
            return self.0.select(request);
        }
        let anchor = request.anchor();
        let candidates = self
            .1
            .iter()
            .filter_map(|(a, b)| {
                if a == anchor {
                    Some(b.clone())
                } else if b == anchor {
                    Some(a.clone())
                } else {
                    None
                }
            })
            .filter(|candidate| request.candidate_universe().contains(candidate))
            .collect();
        CompleteRelationshipSelection::try_new(
            request.clone(),
            candidates,
            vec![Evidence::exact(source(), format!("merges:{anchor}"))],
        )
    }
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

fn strings(values: &[&str]) -> ParameterValue {
    ParameterValue::StringList {
        value: values.iter().map(|value| (*value).to_owned()).collect(),
    }
}

fn number(value: f64) -> ParameterValue {
    ParameterValue::Number { value }
}

fn findings(outcome: &CapabilityEvaluation) -> Vec<(String, String)> {
    outcome
        .findings()
        .iter()
        .map(|finding| {
            let axioval::ir::Scope::Object(object) = &finding.scope else {
                panic!("an object finding: {finding:?}");
            };
            (object.local_id.clone(), finding.message.clone())
        })
        .collect()
}

fn unevaluated(outcome: &CapabilityEvaluation) -> Vec<(String, NotEvaluatedReason, String)> {
    outcome
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| {
            (
                outcome
                    .object_id()
                    .map_or_else(String::new, |id| id.local_id.clone()),
                outcome.reason().clone(),
                outcome.message().to_owned(),
            )
        })
        .collect()
}

#[test]
fn a_wc_behind_a_gap_narrower_than_the_path_is_unreachable() {
    let outcome = Scene::partitioned(3.3).check(&[]);
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
    assert_eq!(
        findings(&outcome),
        [(
            "wc".to_owned(),
            "no entrance of cad:model/room reaches it on a path 0.9 m wide".to_owned()
        )]
    );
    let finding = &outcome.findings()[0];
    assert_eq!(finding.related, [id("door"), id("room")]);
    assert!(
        finding
            .evidence
            .iter()
            .any(|item| item.locator == "axiolid:circulation:room"),
        "{finding:#?}"
    );
    // A narrower path passes the gap.
    let outcome = Scene::partitioned(3.3).check(&[("width_metres", number(0.6))]);
    assert!(outcome.findings().is_empty(), "{outcome:#?}");
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
}

#[test]
fn a_wc_behind_a_wide_gap_is_reached() {
    let outcome = Scene::partitioned(2.5).check(&[]);
    assert!(outcome.findings().is_empty(), "{outcome:#?}");
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
}

#[test]
fn a_door_swinging_across_the_path_cuts_the_wc_off_once_subtracted() {
    // 1.1 m are left north of the partition, until a cupboard door hinged
    // on the north wall at x 2.6 swings south over the gap.
    let scene = || {
        Scene::partitioned(2.9)
            .body(
                "cupboard",
                "cupboard",
                cuboid([2.6, 4.0, 0.0], [3.5, 4.1, 2.0]),
            )
            .swinging(
                "cupboard",
                [2.6, 4.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, -1.0, 0.0],
            )
    };
    let outcome = scene().check(&[]);
    assert!(outcome.findings().is_empty(), "{outcome:#?}");
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
    let outcome = scene().check(&[("subtract_door_swings", selector(kind("cupboard")))]);
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
    assert_eq!(
        findings(&outcome),
        [(
            "wc".to_owned(),
            "no entrance of cad:model/room reaches it on a path 0.9 m wide".to_owned()
        )]
    );
    // A door whose leaves are unknown leaves the room not evaluated.
    let outcome = Scene::partitioned(2.9)
        .body(
            "hatch",
            "cupboard",
            cuboid([2.6, 4.0, 0.0], [3.5, 4.1, 2.0]),
        )
        .check(&[("subtract_door_swings", selector(kind("cupboard")))]);
    assert!(outcome.findings().is_empty(), "{outcome:#?}");
    assert_eq!(
        unevaluated(&outcome)[0].1,
        NotEvaluatedReason::IncompleteEvidence
    );
}

#[test]
fn a_gap_exactly_as_wide_as_the_path_is_not_evaluated() {
    let outcome = Scene::partitioned(3.1).check(&[]);
    assert!(outcome.findings().is_empty(), "{outcome:#?}");
    let open = unevaluated(&outcome);
    assert_eq!(open.len(), 1, "{outcome:#?}");
    assert_eq!(open[0].0, "wc");
    assert_eq!(open[0].1, NotEvaluatedReason::IncompleteEvidence);
}

/// A block fills the room east of x 2 and north of y 1.2, leaving a dead-end
/// corridor 1.2 m wide along the south wall, about 4 m long.
fn blocked() -> Scene {
    Scene::room().body("block", "cabinet", cuboid([2.0, 1.2, 0.0], [6.0, 4.0, 2.0]))
}

fn turning() -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("end_width_metres", number(1.5)),
        ("end_length_metres", number(1.5)),
    ]
}

#[test]
fn a_dead_end_too_narrow_to_turn_in_is_found() {
    let outcome = blocked().check(&turning());
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
    let found = findings(&outcome);
    assert_eq!(found.len(), 1, "{outcome:#?}");
    assert_eq!(found[0].0, "room");
    assert!(
        found[0]
            .1
            .starts_with("no free area 1.5 m by 1.5 m lies within 0.75 m of the path end at (5."),
        "{found:?}"
    );
    assert_eq!(outcome.findings()[0].related, [id("door")]);
}

#[test]
fn short_and_narrow_ends_need_no_free_area() {
    let with = |extra: (&'static str, ParameterValue)| {
        let mut parameters = turning();
        parameters.push(extra);
        blocked().check(&parameters)
    };
    // The corridor is 1.2 m wide: narrower than 1.3 m, not than 1.1 m.
    let outcome = with(("narrow_end_metres", number(1.3)));
    assert!(outcome.findings().is_empty(), "{outcome:#?}");
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
    assert_eq!(with(("narrow_end_metres", number(1.1))).findings().len(), 1);
    // The free area is one L-shaped path without a junction, so each end's
    // branch is the whole path: about 7.5 m from the bay's end wall round
    // to the corridor's.
    let outcome = with(("short_end_metres", number(10.0)));
    assert!(outcome.findings().is_empty(), "{outcome:#?}");
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
    assert_eq!(with(("short_end_metres", number(2.5))).findings().len(), 1);
}

#[test]
fn declarations_fail_closed() {
    let outcome = blocked().check(&[("end_width_metres", number(1.5))]);
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::InvalidDeclaration
    );
    let outcome = blocked().check(&[("short_end_metres", number(1.5))]);
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::InvalidDeclaration
    );
    let outcome = blocked().check(&[(
        "component_mode",
        ParameterValue::String {
            value: "near".into(),
        },
    )]);
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::InvalidDeclaration
    );
}

/// A room 20 m long and 1.8 m wide, cabinets 0.8 m deep along its north
/// wall from x 3 to x 18 and a WC in its north-east corner: a 0.9 m path
/// runs along the cabinets, a 1.5 m square passing space fits only beyond
/// them.
fn corridor() -> Scene {
    Scene {
        objects: Vec::new(),
        geometry: AxiolidGeometry::new(),
        leaves: BTreeMap::new(),
        spaces: Vec::new(),
        merges: Vec::new(),
    }
    .body("room", "room", cuboid([0.0, 0.0, 0.0], [20.0, 1.8, 3.0]))
    .body("door", "door", cuboid([0.5, -0.2, 0.0], [1.5, 0.0, 2.1]))
    .body(
        "cabinets",
        "cabinet",
        cuboid([3.0, 1.0, 0.0], [18.0, 1.8, 2.0]),
    )
    .body("wc", "wc", cuboid([19.2, 1.1, 0.0], [19.8, 1.7, 0.8]))
}

fn passing(spacing: f64) -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("passing_width_metres", number(1.5)),
        ("passing_length_metres", number(1.5)),
        ("passing_spacing_metres", number(spacing)),
    ]
}

#[test]
fn a_path_without_passing_spaces_along_the_cabinets_is_found() {
    let outcome = corridor().check(&passing(10.0));
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
    let found = findings(&outcome);
    assert_eq!(found.len(), 1, "{outcome:#?}");
    assert_eq!(found[0].0, "wc");
    assert!(
        found[0].1.starts_with(
            "the path from cad:model/door has no passing space (1.5 m by 1.5 m) between"
        ),
        "{found:?}"
    );
    assert_eq!(outcome.findings()[0].related, [id("door"), id("room")]);
    // Its ends count as passing spaces, and the whole path is shorter.
    let outcome = corridor().check(&passing(25.0));
    assert!(outcome.findings().is_empty(), "{outcome:#?}");
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
}

/// The room and an annex east of it (x 6..9), sharing the boundary at x 6
/// with nothing between; a bed in the room, a WC in the annex.
fn suite() -> Scene {
    let mut scene = Scene::room()
        .body("annex", "room", cuboid([6.0, 0.0, 0.0], [9.0, 4.0, 3.0]))
        .body("bed", "bed", cuboid([0.5, 2.0, 0.0], [2.5, 3.9, 0.5]))
        .body("wc", "wc", cuboid([8.3, 0.2, 0.0], [8.9, 0.9, 0.8]));
    scene.spaces.push("annex");
    scene.merges.push((id("room"), id("annex")));
    scene
}

fn linking() -> Vec<(&'static str, ParameterValue)> {
    vec![
        (
            "component_selector",
            selector(Selector::AnyOf {
                operands: vec![kind("bed"), kind("wc")],
            }),
        ),
        (
            "component_mode",
            ParameterValue::String {
                value: "link".into(),
            },
        ),
        ("merge_path", strings(&["merges"])),
    ]
}

#[test]
fn a_bed_is_linked_to_a_wc_across_merged_spaces() {
    let outcome = suite().check(&linking());
    assert!(outcome.findings().is_empty(), "{outcome:#?}");
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
    // A wall along the shared boundary parts them.
    let outcome = suite()
        .body("wall", "wall", cuboid([5.9, 0.0, 0.0], [6.0, 4.0, 3.0]))
        .check(&linking());
    let found = findings(&outcome);
    assert!(
        found.iter().any(|(object, message)| object == "bed"
            && message == "no path 0.9 m wide in cad:model/room links it with cad:model/wc"),
        "{found:#?}"
    );
}

#[test]
fn a_low_skirting_is_ignored_with_the_band_starting_above_it() {
    // A skirting 0.1 m high runs across the room at x 3.0..3.1.
    let scene = || {
        Scene::room()
            .body(
                "skirting",
                "skirting",
                cuboid([3.0, 0.0, 0.0], [3.1, 4.0, 0.1]),
            )
            .body("wc", "wc", cuboid([5.3, 0.2, 0.0], [5.9, 0.9, 0.8]))
    };
    let outcome = scene().check(&[]);
    assert_eq!(findings(&outcome).len(), 1, "{outcome:#?}");
    let outcome = scene().check(&[("band_from_metres", number(0.2))]);
    assert!(outcome.findings().is_empty(), "{outcome:#?}");
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
    // A band starting at the clear height is an invalid declaration.
    let outcome = scene().check(&[("band_from_metres", number(2.0))]);
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::InvalidDeclaration
    );
}

#[test]
fn a_path_end_near_a_selected_component_needs_no_free_area() {
    let with = |reach: f64| {
        let mut parameters = turning();
        parameters.push(("end_exempt_selector", selector(kind("cabinet"))));
        parameters.push(("end_exempt_reach_metres", number(reach)));
        blocked().check(&parameters)
    };
    // The dead end lies about 0.6 m from the block.
    let outcome = with(1.5);
    assert!(outcome.findings().is_empty(), "{outcome:#?}");
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
    assert_eq!(with(0.2).findings().len(), 1);
}

#[test]
fn each_component_of_one_set_must_reach_one_of_the_other() {
    let scene = |partition: f64| {
        Scene::partitioned(partition).body("bed", "bed", cuboid([0.5, 2.0, 0.0], [2.5, 3.0, 0.5]))
    };
    let pairing = || {
        vec![
            ("component_selector", selector(kind("bed"))),
            (
                "component_mode",
                ParameterValue::String {
                    value: "link_sets".into(),
                },
            ),
            ("partner_selector", selector(kind("wc"))),
        ]
    };
    let outcome = scene(3.3).check(&pairing());
    assert_eq!(
        findings(&outcome),
        [(
            "bed".to_owned(),
            "no path 0.9 m wide in cad:model/room links it with a partner".to_owned()
        )],
        "{outcome:#?}"
    );
    let outcome = scene(2.5).check(&pairing());
    assert!(outcome.findings().is_empty(), "{outcome:#?}");
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
    // `link_sets` needs its partners.
    let outcome = scene(2.5).check(&[(
        "component_mode",
        ParameterValue::String {
            value: "link_sets".into(),
        },
    )]);
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::InvalidDeclaration
    );
}
