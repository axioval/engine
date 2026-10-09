//! Clash classes, tolerances and exclusions measured on real meshes.
//!
//! The Axiolid proximity service measures each pair; `clash` classifies it.
//! Relationships and presentation layers come from a small in-memory source,
//! so each exclusion can be switched on and off around the same geometry.
#![cfg(feature = "axiolid")]
#![allow(missing_docs)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval::axiolid::{AxiolidGeometry, AxiolidProximityService};
use axioval::engine::{
    CapabilityEvaluation, CompiledRule, CompletePropertyAbsenceEvidence,
    CompleteRelationshipSelection, PropertyRequest, PropertyResolution, PropertyResolutionError,
    PropertyResolutionService, PropertyResolutionServiceHandle, ProximityServiceHandle,
    RelationshipQuery, RelationshipSelectionError, RelationshipSelectionRequest,
    RelationshipSelectionService, RelationshipSelectionServiceHandle, ResolvedProperty,
    RuleCapability, RuleContext, ServiceRegistry, TraversalDirection,
};
use axioval::ir::contract::{ParameterValue, Selector, Severity};
use axioval::ir::{
    Evidence, NotEvaluatedReason, Object, ObjectId, PRESENTATION_LAYER, PRESENTATION_SET, Project,
    Property, PropertyValue, RuleId, SourceId,
};
use axioval::rules::Clash;

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

/// A closed, outward-oriented axis-aligned box.
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
            0, 2, 1, 0, 3, 2, // bottom
            4, 5, 6, 4, 6, 7, // top
            0, 1, 5, 0, 5, 4, // front
            3, 7, 6, 3, 6, 2, // back
            0, 4, 7, 0, 7, 3, // left
            1, 2, 6, 1, 6, 5, // right
        ],
    )
}

/// Objects with meshes, relationship edges and presentation layers.
#[derive(Default)]
struct Scene {
    objects: Vec<Object>,
    geometry: AxiolidGeometry,
    /// relationship -> (relating, related)
    edges: BTreeMap<String, Vec<(ObjectId, ObjectId)>>,
    layers: BTreeMap<ObjectId, Vec<String>>,
    /// The source records no layers for anything.
    no_layers: bool,
}

impl Scene {
    fn body(mut self, local: &str, kind: &str, mesh: TriMesh) -> Self {
        self.objects.push(Object::new(id(local), kind));
        self.geometry = self.geometry.with_mesh(id(local), mesh);
        self
    }
    /// A whole with no body of its own, measured through its `parts`.
    fn whole(mut self, local: &str, kind: &str, parts: &[&str]) -> Self {
        self.objects.push(Object::new(id(local), kind));
        let parts: Vec<ObjectId> = parts.iter().map(|part| id(part)).collect();
        let body = self.geometry.compose(&parts).unwrap();
        self.geometry = self.geometry.with_composed_body(id(local), body);
        self
    }
    /// An object without geometry: a system, an assembly, a port.
    fn node(mut self, local: &str, kind: &str) -> Self {
        self.objects.push(Object::new(id(local), kind));
        self
    }
    fn edge(mut self, relationship: &str, relating: &str, related: &str) -> Self {
        self.edges
            .entry(relationship.into())
            .or_default()
            .push((id(relating), id(related)));
        self
    }
    fn layer(mut self, local: &str, layer: &str) -> Self {
        self.layers.entry(id(local)).or_default().push(layer.into());
        self
    }
    fn without_layers(mut self) -> Self {
        self.no_layers = true;
        self
    }

    fn check(self, parameters: &[(&str, ParameterValue)]) -> CapabilityEvaluation {
        let mut bound: BTreeMap<String, ParameterValue> = BTreeMap::from([
            (
                "counterparts".to_owned(),
                ParameterValue::Selector {
                    value: Box::new(kind("wall")),
                },
            ),
            (
                "penetration_tolerance_metres".to_owned(),
                ParameterValue::Number { value: 0.0 },
            ),
        ]);
        for (name, value) in parameters {
            bound.insert((*name).to_owned(), value.clone());
        }
        let rule = CompiledRule {
            id: RuleId::new("clash").unwrap(),
            capability: "axioval:capability.clash".into(),
            severity: Severity::Error,
            selector: kind("pipe"),
            parameters: bound,
        };
        let project = Project::new(self.objects.clone()).unwrap();
        let proximity = AxiolidProximityService::new(self.geometry.clone());
        let semantic = Arc::new(self);
        let mut services = ServiceRegistry::new();
        services
            .register(ProximityServiceHandle::new(Arc::new(proximity)))
            .unwrap();
        services
            .register(PropertyResolutionServiceHandle::new(semantic.clone()))
            .unwrap();
        services
            .register(RelationshipSelectionServiceHandle::new(semantic))
            .unwrap();
        // A template reads its measured lists as a run does, and is held to the
        // implementation it replaced on the same geometry.
        axioval::rules::register_builtins(axioval::engine::CapabilityRegistry::new())
            .unwrap()
            .install_measured(&mut services, &project);
        let context = RuleContext {
            project: &project,
            services: &services,
        };
        let evaluated = Clash.evaluate(&context, &rule);
        let replaced = axioval_rules::reference::Clash.evaluate(&context, &rule);
        let parity = axioval::rules::parity::Parity::contract().compare(
            (
                "clash",
                &axioval::rules::parity::Observations::of_evaluation(&replaced),
            ),
            (
                "template",
                &axioval::rules::parity::Observations::of_evaluation(&evaluated),
            ),
        );
        assert!(parity.holds(), "{}", parity.diff());
        evaluated
    }
}

impl PropertyResolutionService for Scene {
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        assert_eq!(request.property_set(), Some(PRESENTATION_SET));
        assert_eq!(request.property(), PRESENTATION_LAYER);
        if self.no_layers {
            return Err(PropertyResolutionError::NotRecorded(
                "the source assigns no presentation layers".into(),
            ));
        }
        let object = request.object_id();
        match self.layers.get(object) {
            Some(layers) => {
                let value = PropertyValue::List(
                    layers.iter().cloned().map(PropertyValue::String).collect(),
                );
                let property = Property::new(PRESENTATION_SET, PRESENTATION_LAYER, value)
                    .unwrap()
                    .with_evidence(Evidence::exact(source(), format!("layers:{object}")));
                Ok(PropertyResolution::Present(ResolvedProperty::try_new(
                    request.clone(),
                    property,
                )?))
            }
            None => Ok(PropertyResolution::Absent(
                CompletePropertyAbsenceEvidence::try_new(
                    request.clone(),
                    Evidence::exact(source(), format!("no-layers:{object}")),
                )?,
            )),
        }
    }
}

impl RelationshipSelectionService for Scene {
    fn select(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        let RelationshipQuery::Related {
            relationship,
            direction,
            ..
        } = request.query()
        else {
            return Err(RelationshipSelectionError::InvalidRequest);
        };
        let Some(edges) = self.edges.get(relationship.as_str()) else {
            return Err(RelationshipSelectionError::Unavailable(format!(
                "no {} in this source",
                relationship.as_str()
            )));
        };
        let anchor = request.anchor();
        let reached: BTreeSet<ObjectId> = edges
            .iter()
            .filter_map(|(relating, related)| {
                let forward = matches!(
                    direction,
                    TraversalDirection::Forward | TraversalDirection::Either
                );
                let backward = matches!(
                    direction,
                    TraversalDirection::Backward | TraversalDirection::Either
                );
                if forward && relating == anchor {
                    Some(related.clone())
                } else if backward && related == anchor {
                    Some(relating.clone())
                } else {
                    None
                }
            })
            .filter(|found| request.candidate_universe().contains(found))
            .collect();
        CompleteRelationshipSelection::try_new(
            request.clone(),
            reached.into_iter().collect(),
            vec![Evidence::exact(
                source(),
                format!("scan:{}", relationship.as_str()),
            )],
        )
    }
}

fn kind(kind: &str) -> Selector {
    Selector::EntityType {
        object_type: kind.into(),
        include_subtypes: false,
    }
}

fn number(value: f64) -> ParameterValue {
    ParameterValue::Number { value }
}

fn off() -> ParameterValue {
    ParameterValue::Boolean { value: false }
}

fn paths(values: &[&str]) -> ParameterValue {
    ParameterValue::StringList {
        value: values.iter().map(|value| (*value).to_owned()).collect(),
    }
}

/// `(subject, counterpart, message)` of every finding, sorted.
fn findings(outcome: &CapabilityEvaluation) -> Vec<(String, String, String)> {
    let mut found: Vec<_> = outcome
        .findings()
        .iter()
        .map(|finding| {
            (
                finding.object_id().unwrap().local_id.clone(),
                finding.related[0].local_id.clone(),
                finding.message.clone(),
            )
        })
        .collect();
    found.sort();
    found
}

/// `(object, reason)` of every not-evaluated outcome.
fn open(outcome: &CapabilityEvaluation) -> Vec<(String, NotEvaluatedReason)> {
    outcome
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| {
            (
                outcome.object_id().unwrap().local_id.clone(),
                outcome.reason().clone(),
            )
        })
        .collect()
}

fn wall() -> TriMesh {
    cuboid([0.0, 0.0, 0.0], [4.0, 0.2, 3.0])
}

/// A 0.1 m square pipe through the wall, and one resting on its top.
fn pipes_at_a_wall() -> Scene {
    Scene::default()
        .body("wall", "wall", wall())
        .body("through", "pipe", cuboid([1.0, -1.0, 1.0], [1.1, 1.2, 1.1]))
        .body("resting", "pipe", cuboid([2.0, -1.0, 3.0], [2.1, 1.2, 3.1]))
}

fn only(outcome: &CapabilityEvaluation) -> (String, String, String) {
    assert!(open(outcome).is_empty(), "{:?}", open(outcome));
    let [finding] = findings(outcome)
        .try_into()
        .unwrap_or_else(|found: Vec<_>| {
            panic!("one finding expected: {found:?}");
        });
    finding
}

#[test]
fn an_intersection_is_found_and_a_resting_pipe_is_not() {
    let (subject, counterpart, message) = only(&pipes_at_a_wall().check(&[]));
    assert_eq!(
        (subject.as_str(), counterpart.as_str()),
        ("through", "wall")
    );
    assert!(
        message.starts_with("hard clash with") && message.contains("penetration 0.1000 m"),
        "{message}"
    );
}

#[test]
fn a_copy_within_the_duplicate_tolerance_is_a_duplicate() {
    let scene = Scene::default().body("wall", "wall", wall()).body(
        "copy",
        "pipe",
        cuboid([0.003, 0.0, 0.0], [4.003, 0.2, 3.0]),
    );
    let (_, _, message) = only(&scene.check(&[("duplicate_tolerance_metres", number(0.005))]));
    assert!(message.starts_with("duplicate of"), "{message}");

    // Beyond the tolerance the copy is an intersection.
    let scene = Scene::default().body("wall", "wall", wall()).body(
        "copy",
        "pipe",
        cuboid([0.003, 0.0, 0.0], [4.003, 0.2, 3.0]),
    );
    let (_, _, message) = only(&scene.check(&[("duplicate_tolerance_metres", number(0.001))]));
    assert!(message.starts_with("hard clash"), "{message}");

    // Switched off, the duplicate is not reported as anything else.
    let scene = Scene::default().body("wall", "wall", wall()).body(
        "copy",
        "pipe",
        cuboid([0.003, 0.0, 0.0], [4.003, 0.2, 3.0]),
    );
    let outcome = scene.check(&[
        ("duplicate_tolerance_metres", number(0.005)),
        ("report_duplicates", off()),
    ]);
    assert!(findings(&outcome).is_empty() && open(&outcome).is_empty());
}

#[test]
fn a_body_inside_another_is_contained_unless_switched_off() {
    let scene = || {
        Scene::default()
            .body("wall", "wall", cuboid([0.0, 0.0, 0.0], [4.0, 4.0, 3.0]))
            .body("sleeve", "pipe", cuboid([1.0, 1.0, 1.0], [2.0, 2.0, 2.0]))
    };
    let (_, _, message) = only(&scene().check(&[]));
    assert!(message.starts_with("lies wholly inside"), "{message}");
    let outcome = scene().check(&[("report_containment", off())]);
    assert!(findings(&outcome).is_empty() && open(&outcome).is_empty());
}

#[test]
fn intersections_can_be_switched_off() {
    let outcome = pipes_at_a_wall().check(&[("report_intersections", off())]);
    assert!(findings(&outcome).is_empty() && open(&outcome).is_empty());
}

/// The pipe crosses the wall 0.1 m wide in plan (its narrower plan axis) and
/// 0.1 m high: a tolerance above either hides it.
#[test]
fn axis_tolerances_hide_narrow_intersections() {
    for (horizontal, vertical, reported) in [
        (0.05, 0.05, true),
        (0.12, 0.0, false),
        (0.0, 0.15, false),
        (0.09, 0.09, true),
    ] {
        let outcome = pipes_at_a_wall().check(&[
            ("horizontal_tolerance_metres", number(horizontal)),
            ("vertical_tolerance_metres", number(vertical)),
        ]);
        assert!(open(&outcome).is_empty(), "{:?}", open(&outcome));
        assert_eq!(
            !findings(&outcome).is_empty(),
            reported,
            "{horizontal}/{vertical}: {:?}",
            findings(&outcome)
        );
    }
}

/// A duct sunk 5 mm into a slab: wide in plan, shallow in height.
#[test]
fn a_shallow_overlap_passes_the_vertical_tolerance() {
    let scene = || {
        Scene::default()
            .body("slab", "wall", cuboid([0.0, 0.0, 3.0], [4.0, 4.0, 3.2]))
            .body("duct", "pipe", cuboid([1.0, 1.0, 2.5], [3.0, 1.5, 3.005]))
    };
    let outcome = scene().check(&[("vertical_tolerance_metres", number(0.01))]);
    assert!(findings(&outcome).is_empty() && open(&outcome).is_empty());
    let (_, _, message) = only(&scene().check(&[("vertical_tolerance_metres", number(0.001))]));
    assert!(message.contains("0.0050 m vertically"), "{message}");
}

#[test]
fn pairs_in_the_same_system_are_excluded() {
    let scene = pipes_at_a_wall()
        .node("heating", "system")
        .edge("IfcRelAssignsToGroup", "heating", "through")
        .edge("IfcRelAssignsToGroup", "heating", "wall");
    let outcome = scene.check(&[("exclude_paths", paths(&["IfcRelAssignsToGroup:backward"]))]);
    assert!(findings(&outcome).is_empty() && open(&outcome).is_empty());

    // Another system shares nothing.
    let scene = pipes_at_a_wall()
        .node("heating", "system")
        .node("structure", "system")
        .edge("IfcRelAssignsToGroup", "heating", "through")
        .edge("IfcRelAssignsToGroup", "structure", "wall");
    let outcome = scene.check(&[("exclude_paths", paths(&["IfcRelAssignsToGroup:backward"]))]);
    assert_eq!(findings(&outcome).len(), 1);
}

#[test]
fn parts_of_the_same_element_are_excluded() {
    let scene = pipes_at_a_wall()
        .node("assembly", "assembly")
        .edge("IfcRelAggregates", "assembly", "through")
        .edge("IfcRelAggregates", "assembly", "wall");
    let outcome = scene.check(&[("exclude_paths", paths(&["IfcRelAggregates:backward"]))]);
    assert!(findings(&outcome).is_empty() && open(&outcome).is_empty());
}

/// A pipe run measured through its two segments clashes with the wall as
/// each segment does, and never with its own segments: one body is not two.
#[test]
fn a_whole_is_never_paired_with_its_own_parts() {
    let scene = Scene::default()
        .body("wall", "pipe", wall())
        .body(
            "segment-a",
            "pipe",
            cuboid([1.0, -1.0, 1.0], [1.1, 0.1, 1.1]),
        )
        .body(
            "segment-b",
            "pipe",
            cuboid([1.0, 0.1, 1.0], [1.1, 1.2, 1.1]),
        )
        .whole("run", "pipe", &["segment-a", "segment-b"]);
    let outcome = scene.check(&[(
        "counterparts",
        ParameterValue::Selector {
            value: Box::new(kind("pipe")),
        },
    )]);
    let mut pairs: Vec<(String, String)> = findings(&outcome)
        .into_iter()
        .map(|(subject, counterpart, _)| {
            if subject < counterpart {
                (subject, counterpart)
            } else {
                (counterpart, subject)
            }
        })
        .collect();
    pairs.sort();
    pairs.dedup();
    assert_eq!(
        pairs,
        [
            ("run".to_owned(), "wall".to_owned()),
            ("segment-a".to_owned(), "wall".to_owned()),
            ("segment-b".to_owned(), "wall".to_owned()),
        ],
        "{outcome:#?}"
    );
    assert!(open(&outcome).is_empty(), "{:?}", open(&outcome));
}

/// Pipe to its port, across the port connection, to the wall's port and on
/// to the wall: the path reaches the counterpart itself.
#[test]
fn elements_connected_by_ports_are_excluded() {
    let path = "IfcRelConnectsPortToElement:backward IfcRelConnectsPorts:either IfcRelConnectsPortToElement:forward";
    let scene = pipes_at_a_wall()
        .node("pipe-port", "port")
        .node("wall-port", "port")
        .edge("IfcRelConnectsPortToElement", "pipe-port", "through")
        .edge("IfcRelConnectsPortToElement", "wall-port", "wall")
        .edge("IfcRelConnectsPorts", "pipe-port", "wall-port");
    let outcome = scene.check(&[("exclude_paths", paths(&[path]))]);
    assert!(findings(&outcome).is_empty() && open(&outcome).is_empty());

    // Unconnected ports exclude nothing.
    let scene = pipes_at_a_wall()
        .node("pipe-port", "port")
        .node("wall-port", "port")
        .edge("IfcRelConnectsPortToElement", "pipe-port", "through")
        .edge("IfcRelConnectsPortToElement", "wall-port", "wall")
        .edge("IfcRelConnectsPorts", "pipe-port", "pipe-port");
    let outcome = scene.check(&[("exclude_paths", paths(&[path]))]);
    assert_eq!(findings(&outcome).len(), 1, "{:?}", open(&outcome));
}

#[test]
fn pairs_on_a_shared_layer_are_excluded() {
    let scene = pipes_at_a_wall()
        .layer("through", "M-PIPE")
        .layer("through", "S-WALL")
        .layer("wall", "S-WALL");
    let outcome = scene.check(&[(
        "exclude_same_layer",
        ParameterValue::Boolean { value: true },
    )]);
    assert!(findings(&outcome).is_empty() && open(&outcome).is_empty());

    let scene = pipes_at_a_wall()
        .layer("through", "M-PIPE")
        .layer("wall", "S-WALL");
    let outcome = scene.check(&[(
        "exclude_same_layer",
        ParameterValue::Boolean { value: true },
    )]);
    assert_eq!(findings(&outcome).len(), 1);
}

/// An exclusion that cannot be decided never hides a clash, and never
/// reports one either.
#[test]
fn an_undecided_exclusion_leaves_the_clash_not_evaluated() {
    let outcome = pipes_at_a_wall().without_layers().check(&[(
        "exclude_same_layer",
        ParameterValue::Boolean { value: true },
    )]);
    assert!(findings(&outcome).is_empty());
    assert_eq!(
        open(&outcome),
        vec![("through".to_owned(), NotEvaluatedReason::NotRecorded)]
    );

    let outcome = pipes_at_a_wall().check(&[("exclude_paths", paths(&["IfcRelNests:backward"]))]);
    assert!(findings(&outcome).is_empty());
    assert_eq!(
        open(&outcome),
        vec![("through".to_owned(), NotEvaluatedReason::BackendUnavailable)]
    );
}

#[test]
fn a_malformed_exclusion_path_is_an_invalid_declaration() {
    let outcome = pipes_at_a_wall().check(&[("exclude_paths", paths(&["   "]))]);
    assert!(
        open(&outcome)
            .iter()
            .all(|(_, reason)| *reason == NotEvaluatedReason::InvalidDeclaration)
    );
    assert!(!open(&outcome).is_empty());
}

/// A counter-clockwise profile in (x, z), its caps triangulated by `caps`,
/// extruded over y from `y0` to `y1`: a closed, outward mesh.
fn extruded(profile: &[[f64; 2]], caps: &[[u32; 3]], y0: f64, y1: f64) -> TriMesh {
    let n = u32::try_from(profile.len()).unwrap();
    let mut positions: Vec<Point3> = profile
        .iter()
        .map(|[x, z]| Point3::new(*x, y0, *z))
        .collect();
    positions.extend(profile.iter().map(|[x, z]| Point3::new(*x, y1, *z)));
    let mut indices = Vec::new();
    for [a, b, c] in caps {
        indices.extend([*a, *b, *c, a + n, c + n, b + n]);
    }
    for i in 0..n {
        let j = (i + 1) % n;
        indices.extend([i, j + n, j, i, i + n, j + n]);
    }
    TriMesh::new(positions, indices)
}

/// A layer 4 m long, 3 m high and 0.1 m thick with a door hole 1 m wide and
/// 2.1 m high already cut, and the opening box spanning x `x0..x1` through
/// it. The layer's vertex centroid lies in the hole.
fn layer_and_opening(x0: f64, x1: f64) -> (TriMesh, TriMesh) {
    let layer = extruded(
        &[
            [0.0, 0.0],
            [1.5, 0.0],
            [1.5, 2.1],
            [2.5, 2.1],
            [2.5, 0.0],
            [4.0, 0.0],
            [4.0, 3.0],
            [0.0, 3.0],
        ],
        &[
            [0, 1, 2],
            [0, 2, 7],
            [2, 3, 7],
            [3, 6, 7],
            [3, 4, 6],
            [4, 5, 6],
        ],
        -0.1,
        0.0,
    );
    let opening = extruded(
        &[[x0, 0.0], [x1, 0.0], [x1, 2.1], [x0, 2.1]],
        &[[0, 1, 2], [0, 2, 3]],
        -0.2,
        0.1,
    );
    (layer, opening)
}

/// A door filling the hole already cut in its wall is no clash: the wall's
/// centroid lies in the hole, inside the door, but is no point of the wall.
/// A door reaching past the hole's edge is a hard clash as deep as it
/// reaches (#315).
#[test]
fn a_door_filling_its_walls_hole_is_no_clash() {
    let (wall, door) = layer_and_opening(1.5, 2.5);
    let outcome = Scene::default()
        .body("wall", "wall", wall)
        .body("door", "pipe", door)
        .check(&[("penetration_tolerance_metres", number(0.01))]);
    assert!(
        findings(&outcome).is_empty() && open(&outcome).is_empty(),
        "{:?} {:?}",
        findings(&outcome),
        open(&outcome)
    );
    let (wall, door) = layer_and_opening(1.45, 2.5);
    let (_, _, message) = only(
        &Scene::default()
            .body("wall", "wall", wall)
            .body("door", "pipe", door)
            .check(&[("penetration_tolerance_metres", number(0.01))]),
    );
    assert!(
        message.starts_with("hard clash with") && message.contains("penetration 0.0500 m"),
        "{message}"
    );
}
