//! Free space in front of and beside components, measured on real meshes.
//!
//! Axiolid measures the extents and the clearance; object frames and the
//! containing space come from a small in-memory source, so the rule's
//! stated front and side decide where the volume stands.
#![cfg(feature = "axiolid")]
#![allow(missing_docs)]

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval::axiolid::{
    AxiolidFreeSpaceService, AxiolidGeometry, AxiolidPlanSpanService, AxiolidVerticalExtentService,
};
use axioval::engine::{
    CapabilityEvaluation, CompiledRule, CompletePropertyAbsenceEvidence,
    CompleteRelationshipSelection, FreeSpaceServiceHandle, MetricDirection, MetricFrame,
    MetricPoint, ObjectFrame, ObjectFrameError, ObjectFrameService, ObjectFrameServiceHandle,
    ObjectFront, PlanSpanServiceHandle, PropertyRequest, PropertyResolution,
    PropertyResolutionError, PropertyResolutionService, PropertyResolutionServiceHandle,
    RelationshipQuery, RelationshipSelectionError, RelationshipSelectionRequest,
    RelationshipSelectionService, RelationshipSelectionServiceHandle, ResolvedProperty,
    RuleCapability, RuleContext, ServiceRegistry, SourceSnapshot, TraversalDirection,
    VerticalExtentServiceHandle,
};
use axioval::ir::contract::{ParameterValue, Selector, Severity};
use axioval::ir::{
    Evidence, NotEvaluatedReason, Object, ObjectId, Project, Property, PropertyValue,
    QuantityDimension, RuleId, SourceId,
};
use axioval::rules::ComponentClearance;

const IN_SPACE: &str = "in-space";

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

/// A closed, outward-oriented box, turned by `turn` radians about the
/// vertical through `pivot`.
fn turned(min: [f64; 3], max: [f64; 3], turn: f64, pivot: [f64; 2]) -> TriMesh {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let (sin, cos) = turn.sin_cos();
    let at = |x: f64, y: f64, z: f64| {
        let (dx, dy) = (x - pivot[0], y - pivot[1]);
        Point3::new(
            pivot[0] + cos * dx - sin * dy,
            pivot[1] + sin * dx + cos * dy,
            z,
        )
    };
    TriMesh::new(
        vec![
            at(x0, y0, z0),
            at(x1, y0, z0),
            at(x1, y1, z0),
            at(x0, y1, z0),
            at(x0, y0, z1),
            at(x1, y0, z1),
            at(x1, y1, z1),
            at(x0, y1, z1),
        ],
        vec![
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7,
            3, 1, 2, 6, 1, 6, 5,
        ],
    )
}

fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
    turned(min, max, 0.0, [0.0, 0.0])
}

/// A 3 x 3 m room with a WC against its south wall: the WC is 0.4 m wide
/// (x 1.0 to 1.4), 0.7 m deep (y 0 to 0.7) and 0.4 m high, and faces north,
/// along its placement's forward axis. Facing north, its left is west.
struct Scene {
    objects: Vec<Object>,
    geometry: AxiolidGeometry,
    snapshots: Vec<SourceSnapshot>,
    /// The WC's right axis; forward is up × right.
    right: [f64; 3],
    front: ObjectFront,
    edges: Vec<(ObjectId, ObjectId)>,
    /// The WC's stated `Door.ClearWidth`, standing in for a door's.
    clear_width: Option<PropertyValue>,
}

impl Scene {
    fn wc() -> Self {
        Self {
            objects: Vec::new(),
            geometry: AxiolidGeometry::new(),
            snapshots: vec![SourceSnapshot::try_new(source(), "r1", "sha256:1").unwrap()],
            right: [1.0, 0.0, 0.0],
            front: ObjectFront::NotStated,
            edges: vec![(id("wc"), id("room"))],
            clear_width: None,
        }
        .body("room", "space", cuboid([0.0, 0.0, 0.0], [3.0, 3.0, 2.5]))
        .body("wc", "wc", cuboid([1.0, 0.0, 0.0], [1.4, 0.7, 0.4]))
    }

    fn body(mut self, local: &str, kind: &str, mesh: TriMesh) -> Self {
        self.objects.push(Object::new(id(local), kind));
        self.geometry = self.geometry.with_mesh(id(local), mesh);
        self
    }

    /// Replaces the WC's body and frame with ones turned by `turn`.
    fn turned_wc(mut self, turn: f64) -> Self {
        let pivot = [1.2, 0.35];
        self.geometry = self.geometry.with_mesh(
            id("wc"),
            turned([1.0, 0.0, 0.0], [1.4, 0.7, 0.4], turn, pivot),
        );
        self.right = [turn.cos(), turn.sin(), 0.0];
        self
    }

    fn stated(mut self, front: [f64; 3]) -> Self {
        self.front = ObjectFront::Stated(MetricDirection::try_new(front).unwrap());
        self
    }

    fn check(self, parameters: &[(&str, ParameterValue)]) -> CapabilityEvaluation {
        let mut bound = std::collections::BTreeMap::from([
            ("side".to_owned(), text("left")),
            ("front_axis".to_owned(), text("forward")),
            ("width".to_owned(), metres(0.7)),
            ("depth".to_owned(), metres(0.9)),
            ("height".to_owned(), metres(2.0)),
            ("align".to_owned(), text("right")),
            ("height_reference".to_owned(), text("floor")),
            (
                "space_path".to_owned(),
                ParameterValue::StringList {
                    value: vec![format!("{IN_SPACE}:forward")],
                },
            ),
            (
                "obstacles".to_owned(),
                selector(Selector::Not {
                    operand: Box::new(kind("space")),
                }),
            ),
        ]);
        for (name, value) in parameters {
            match value {
                ParameterValue::StringList { value } if value.is_empty() => {
                    bound.remove(*name);
                }
                _ => {
                    bound.insert((*name).to_owned(), value.clone());
                }
            }
        }
        let rule = CompiledRule {
            id: RuleId::new("clearance").unwrap(),
            capability: "axioval:capability.component-clearance".into(),
            severity: Severity::Error,
            selector: kind("wc"),
            parameters: bound,
        };
        let project = Project::new(self.objects.clone()).unwrap();
        let geometry = self.geometry.clone();
        let semantic = Arc::new(self);
        let mut services = ServiceRegistry::new();
        services
            .register(FreeSpaceServiceHandle::new(Arc::new(
                AxiolidFreeSpaceService::new(geometry.clone(), source()),
            )))
            .unwrap();
        services
            .register(PlanSpanServiceHandle::new(Arc::new(
                AxiolidPlanSpanService::new(geometry.clone(), source()),
            )))
            .unwrap();
        services
            .register(VerticalExtentServiceHandle::new(Arc::new(
                AxiolidVerticalExtentService::new(geometry),
            )))
            .unwrap();
        services
            .register(ObjectFrameServiceHandle::new(semantic.clone()))
            .unwrap();
        services
            .register(PropertyResolutionServiceHandle::new(semantic.clone()))
            .unwrap();
        services
            .register(RelationshipSelectionServiceHandle::new(semantic))
            .unwrap();
        // The template reads its measured checks as a run does.
        axioval::rules::register_builtins(axioval::engine::CapabilityRegistry::new())
            .unwrap()
            .install_measured(&mut services, &project);
        let context = RuleContext {
            project: &project,
            services: &services,
        };
        let template = ComponentClearance.evaluate(&context, &rule);
        // Held to the implementation it replaced, on the same scene.
        let reference = axioval_rules::reference::ComponentClearance.evaluate(&context, &rule);
        let parity = axioval::rules::parity::Parity::contract().compare(
            (
                "component-clearance",
                &axioval::rules::parity::Observations::of_evaluation(&reference),
            ),
            (
                "template",
                &axioval::rules::parity::Observations::of_evaluation(&template),
            ),
        );
        assert!(parity.holds(), "{}", parity.diff());
        template
    }
}

impl ObjectFrameService for Scene {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn object_frame(&self, object: &ObjectId) -> Result<ObjectFrame, ObjectFrameError> {
        let right = self.right;
        let frame = MetricFrame::try_new(
            MetricPoint::try_new(object.clone(), [1.2, 0.0, 0.0]).unwrap(),
            MetricDirection::try_new(right).unwrap(),
            MetricDirection::try_new([-right[1], right[0], 0.0]).unwrap(),
            MetricDirection::try_new([0.0, 0.0, 1.0]).unwrap(),
        )
        .unwrap();
        ObjectFrame::try_new(
            object.clone(),
            frame,
            self.front,
            Evidence::exact(source(), format!("placement:{}", object.local_id)),
        )
    }
}

impl PropertyResolutionService for Scene {
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        match (&self.clear_width, request.property()) {
            (Some(value), "ClearWidth") => {
                let property = Property::new("Door", "ClearWidth", value.clone())
                    .unwrap()
                    .with_evidence(Evidence::exact(source(), "Door.ClearWidth"));
                Ok(PropertyResolution::Present(ResolvedProperty::try_new(
                    request.clone(),
                    property,
                )?))
            }
            _ => Ok(PropertyResolution::Absent(
                CompletePropertyAbsenceEvidence::try_new(
                    request.clone(),
                    Evidence::exact(source(), "absent"),
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
            direction: TraversalDirection::Forward,
            ..
        } = request.query()
        else {
            return Err(RelationshipSelectionError::InvalidRequest);
        };
        if relationship.as_str() != IN_SPACE {
            return Err(RelationshipSelectionError::Unavailable(format!(
                "no {} in this source",
                relationship.as_str()
            )));
        }
        let reached = self
            .edges
            .iter()
            .filter(|(from, to)| {
                from == request.anchor() && request.candidate_universe().contains(to)
            })
            .map(|(_, to)| to.clone())
            .collect();
        CompleteRelationshipSelection::try_new(
            request.clone(),
            reached,
            vec![Evidence::exact(source(), "scan:in-space")],
        )
    }
}

fn kind(kind: &str) -> Selector {
    Selector::EntityType {
        object_type: kind.into(),
        include_subtypes: false,
    }
}

fn selector(selector: Selector) -> ParameterValue {
    ParameterValue::Selector {
        value: Box::new(selector),
    }
}

fn text(value: &str) -> ParameterValue {
    ParameterValue::String {
        value: value.into(),
    }
}

fn metres(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "m".into(),
    }
}

fn yes() -> ParameterValue {
    ParameterValue::Boolean { value: true }
}

/// Removes a default parameter.
fn none() -> ParameterValue {
    ParameterValue::StringList { value: Vec::new() }
}

/// `(message, related)` of every finding.
fn findings(outcome: &CapabilityEvaluation) -> Vec<(String, Vec<String>)> {
    outcome
        .findings()
        .iter()
        .map(|finding| {
            (
                finding.message.clone(),
                finding
                    .related
                    .iter()
                    .map(|related| related.local_id.clone())
                    .collect(),
            )
        })
        .collect()
}

fn unevaluated(outcome: &CapabilityEvaluation) -> Vec<(NotEvaluatedReason, String)> {
    outcome
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| (outcome.reason().clone(), outcome.message().to_owned()))
        .collect()
}

fn clean(outcome: &CapabilityEvaluation) -> bool {
    outcome.findings().is_empty() && outcome.not_evaluated_outcomes().is_empty()
}

/// A basin on the wall west of the WC, 0.8 to 1 m above the floor, inside
/// the transfer area (x 0.1 to 1, y 0 to 0.7).
fn basin(scene: Scene) -> Scene {
    scene.body("basin", "basin", cuboid([0.3, 0.2, 0.8], [0.6, 0.5, 1.0]))
}

#[test]
fn a_basin_in_the_transfer_area_obstructs_it() {
    let outcome = basin(Scene::wc()).check(&[]);
    assert_eq!(
        findings(&outcome),
        [(
            "left clearance (0.7 m wide, 0.9 m deep, 2 m high) is obstructed by cad:model/basin"
                .to_owned(),
            vec!["basin".to_owned()]
        )],
        "{outcome:#?}"
    );
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
}

#[test]
fn a_transfer_area_with_the_basin_elsewhere_is_clear() {
    let scene = Scene::wc().body("basin", "basin", cuboid([1.6, 0.2, 0.8], [1.9, 0.5, 1.0]));
    assert!(clean(&scene.check(&[])));
    // The same basin blocks the right side, which `both_sides` adds.
    let scene = Scene::wc().body("basin", "basin", cuboid([1.6, 0.2, 0.8], [1.9, 0.5, 1.0]));
    let outcome = scene.check(&[("both_sides", yes())]);
    let found = findings(&outcome);
    assert_eq!(found.len(), 1, "{outcome:#?}");
    assert!(found[0].0.starts_with("right clearance"), "{outcome:#?}");
}

#[test]
fn an_allowed_intruder_does_not_obstruct() {
    let outcome = basin(Scene::wc()).check(&[("allowed_intruders", selector(kind("basin")))]);
    assert!(clean(&outcome), "{outcome:#?}");
}

#[test]
fn an_undecided_obstacle_can_only_obstruct() {
    // Every object but the basin is undecided: the source cannot answer
    // the relationship the second operand walks.
    let obstacles = selector(Selector::AnyOf {
        operands: vec![
            kind("basin"),
            Selector::Related {
                path: vec!["unanswered:forward".into()],
                quantifier: axioval::ir::contract::RelatedQuantifier::default(),
                selector: Box::new(Selector::All),
            },
        ],
    });
    // A decided basin in the area still obstructs it.
    let outcome = basin(Scene::wc()).check(&[("obstacles", obstacles.clone())]);
    assert_eq!(findings(&outcome).len(), 1, "{outcome:#?}");
    assert_eq!(findings(&outcome)[0].1, ["basin"], "{outcome:#?}");
    // Elsewhere, only the undecided room meets the area.
    let scene = Scene::wc().body("basin", "basin", cuboid([1.6, 0.2, 0.8], [1.9, 0.5, 1.0]));
    let outcome = scene.check(&[("obstacles", obstacles)]);
    assert!(outcome.findings().is_empty(), "{outcome:#?}");
    let reasons = unevaluated(&outcome);
    assert_eq!(reasons.len(), 1, "{outcome:#?}");
    assert!(
        reasons[0].1.contains("cannot decide: cad:model/room"),
        "{outcome:#?}"
    );
}

#[test]
fn a_protrusion_within_the_allowance_does_not_obstruct() {
    // A shelf reaching 0.1 m into the area from its far (west) side.
    let shelf = || Scene::wc().body("shelf", "fixture", cuboid([0.0, 0.1, 1.2], [0.2, 0.6, 1.4]));
    assert!(clean(&shelf().check(&[("protrusion", metres(0.15))])));
    let outcome = shelf().check(&[("protrusion", metres(0.05))]);
    assert_eq!(findings(&outcome).len(), 1, "{outcome:#?}");
}

#[test]
fn the_height_reference_lifts_the_volume_above_the_basin() {
    // From the WC's top (0.4 m) plus 0.7 m the volume starts at 1.1 m.
    let outcome = basin(Scene::wc()).check(&[
        ("height_reference", text("top")),
        ("vertical_offset", metres(0.7)),
        ("space_path", none()),
    ]);
    assert!(clean(&outcome), "{outcome:#?}");
    let outcome =
        basin(Scene::wc()).check(&[("height_reference", text("bottom")), ("space_path", none())]);
    assert_eq!(findings(&outcome).len(), 1, "{outcome:#?}");
}

/// A turning circle in front of the WC: radius 0.75 m about (1.2, 1.45),
/// touching its front. A chair `gap` metres east of the circle's centre.
fn turning_circle(gap: f64) -> CapabilityEvaluation {
    Scene::wc()
        .body(
            "chair",
            "fixture",
            cuboid([1.2 + gap, 1.35, 0.0], [2.1, 1.55, 0.9]),
        )
        .check(&[
            ("side", text("front")),
            ("width", none()),
            ("depth", none()),
            ("radius", metres(0.75)),
            ("align", text("centre")),
        ])
}

#[test]
fn a_turning_circle_is_decided_outside_its_polygon_band_only() {
    assert_eq!(findings(&turning_circle(0.74)).len(), 1);
    assert!(clean(&turning_circle(0.76)));
    // Just outside the circle but inside its circumscribed polygon: the
    // chair may or may not reach it, so nothing is decided.
    let outcome = turning_circle(0.7505);
    assert!(outcome.findings().is_empty(), "{outcome:#?}");
    let reasons = unevaluated(&outcome);
    assert_eq!(reasons.len(), 1, "{outcome:#?}");
    assert_eq!(reasons[0].0, NotEvaluatedReason::BackendUnavailable);
    assert!(
        reasons[0].1.starts_with("front clearance: ")
            && reasons[0].1.contains("approximation band"),
        "{outcome:#?}"
    );
}

#[test]
fn the_front_is_stated_by_the_rule_or_the_source_never_inferred() {
    let outcome = basin(Scene::wc()).check(&[("front_axis", text("stated"))]);
    let reasons = unevaluated(&outcome);
    assert_eq!(reasons.len(), 1, "{outcome:#?}");
    assert_eq!(reasons[0].0, NotEvaluatedReason::IncompleteEvidence);
    assert!(reasons[0].1.contains("states no front"), "{outcome:#?}");
    let outcome =
        basin(Scene::wc().stated([0.0, 1.0, 0.0])).check(&[("front_axis", text("stated"))]);
    assert_eq!(findings(&outcome).len(), 1, "{outcome:#?}");
    // Facing south instead, the WC's left is east, where nothing stands.
    let outcome = basin(Scene::wc()).check(&[("front_axis", text("-forward"))]);
    assert!(clean(&outcome), "{outcome:#?}");
}

#[test]
fn a_turned_component_measures_its_volume_as_an_interval() {
    // Turned 30 degrees, every extent is an interval: the volume is judged
    // by the union and the common part of its positions.
    let turn = 30_f64.to_radians();
    let outcome = Scene::wc().turned_wc(turn).check(&[]);
    assert!(clean(&outcome), "{outcome:#?}");
    // A post right beside the WC's west face is in every position.
    let (sin, cos) = turn.sin_cos();
    let [x, y] = [1.2 - 0.6 * cos, 0.35 - 0.6 * sin];
    let outcome = Scene::wc()
        .turned_wc(turn)
        .body(
            "post",
            "fixture",
            cuboid([x - 0.05, y - 0.05, 0.0], [x + 0.05, y + 0.05, 1.0]),
        )
        .check(&[]);
    assert_eq!(findings(&outcome).len(), 1, "{outcome:#?}");
    assert!(
        outcome.findings()[0]
            .evidence
            .iter()
            .any(|evidence| !evidence.exact),
        "{outcome:#?}"
    );
}

#[test]
fn the_volume_must_lie_inside_the_space_when_asked() {
    assert!(clean(&Scene::wc().check(&[("within_space", yes())])));
    // Moved west, the WC's transfer area reaches through the west wall.
    let mut scene = Scene::wc();
    scene.geometry = scene
        .geometry
        .with_mesh(id("wc"), cuboid([0.5, 0.0, 0.0], [0.9, 0.7, 0.4]));
    let outcome = scene.check(&[("within_space", yes())]);
    assert_eq!(
        findings(&outcome),
        [(
            "left clearance (0.7 m wide, 0.9 m deep, 2 m high) extends outside cad:model/room"
                .to_owned(),
            vec!["room".to_owned()]
        )],
        "{outcome:#?}"
    );
}

#[test]
fn declarations_and_services_fail_closed() {
    let outcome = Scene::wc().check(&[("radius", metres(0.5))]);
    assert_eq!(
        unevaluated(&outcome)[0].0,
        NotEvaluatedReason::InvalidDeclaration
    );
    let outcome = Scene::wc().check(&[("protrusion", metres(0.35))]);
    assert_eq!(
        unevaluated(&outcome)[0].0,
        NotEvaluatedReason::InvalidDeclaration
    );
    let outcome = Scene::wc().check(&[("space_path", none())]);
    assert_eq!(
        unevaluated(&outcome)[0].0,
        NotEvaluatedReason::InvalidDeclaration
    );
    let outcome = Scene::wc().check(&[("side", text("above"))]);
    assert_eq!(
        unevaluated(&outcome)[0].0,
        NotEvaluatedReason::InvalidDeclaration
    );
    // An obstacle without a body the host could measure refuses clear.
    let mut scene = Scene::wc();
    scene.objects.push(Object::new(id("crate"), "fixture"));
    scene.geometry = scene.geometry.with_unmeasured(id("crate"), "no mesh");
    let outcome = scene.check(&[]);
    assert_eq!(
        unevaluated(&outcome)[0].0,
        NotEvaluatedReason::BackendUnavailable,
        "{outcome:#?}"
    );
}

#[test]
fn a_floating_transfer_area_fits_after_sliding_past_the_basin() {
    // Looking out of the WC's left (west) side, right is north. Slid 0.5 m
    // north or more, the area (y 0.5 to 1.2) passes the basin (y 0.2 to 0.5).
    let outcome =
        basin(Scene::wc()).check(&[("slide_from", metres(0.0)), ("slide_to", metres(1.5))]);
    assert!(clean(&outcome), "{outcome:#?}");
    // Within 0.3 m either way it meets the basin wherever it stands, and
    // south of the room it leaves the space.
    let outcome =
        basin(Scene::wc()).check(&[("slide_from", metres(-0.5)), ("slide_to", metres(0.3))]);
    assert_eq!(
        findings(&outcome),
        [(
            "left clearance (0.7 m wide, 0.9 m deep, 2 m high) fits nowhere between -0.5 m and \
             0.3 m across the side in cad:model/room"
                .to_owned(),
            vec!["room".to_owned()]
        )],
        "{outcome:#?}"
    );
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
    // The basin hangs 0.8 m above the floor: a volume 0.8 m high slides
    // under it without moving.
    let outcome = basin(Scene::wc()).check(&[
        ("height", metres(0.8)),
        ("slide_from", metres(-0.5)),
        ("slide_to", metres(0.3)),
    ]);
    assert!(clean(&outcome), "{outcome:#?}");
}

#[test]
fn a_floating_volume_needs_its_spaces_and_both_offsets() {
    let outcome = Scene::wc().check(&[("slide_from", metres(0.0))]);
    assert_eq!(
        unevaluated(&outcome)[0].0,
        NotEvaluatedReason::InvalidDeclaration
    );
    let outcome = Scene::wc().check(&[
        ("slide_from", metres(0.0)),
        ("slide_to", metres(1.0)),
        ("height_reference", text("bottom")),
        ("space_path", none()),
    ]);
    assert_eq!(
        unevaluated(&outcome)[0].0,
        NotEvaluatedReason::InvalidDeclaration
    );
    let outcome = Scene::wc().check(&[("slide_from", metres(1.0)), ("slide_to", metres(0.0))]);
    assert_eq!(
        unevaluated(&outcome)[0].0,
        NotEvaluatedReason::InvalidDeclaration
    );
}

#[test]
fn a_maximum_size_is_exceeded_by_a_larger_free_volume() {
    let maximum = [
        ("size_mode", text("maximum")),
        ("size_tolerance", metres(0.05)),
    ];
    // In the empty room a volume 5 cm larger in any dimension is free.
    let outcome = Scene::wc().check(&maximum);
    let found: Vec<String> = findings(&outcome)
        .into_iter()
        .map(|(message, _)| message)
        .collect();
    assert_eq!(
        found,
        [
            "left clearance (0.75 m wide, 0.9 m deep, 2 m high) is free, so the free volume \
             exceeds the maximum width",
            "left clearance (0.7 m wide, 0.95 m deep, 2 m high) is free, so the free volume \
             exceeds the maximum depth",
            "left clearance (0.7 m wide, 0.9 m deep, 2.05 m high) is free, so the free volume \
             exceeds the maximum height",
        ],
        "{outcome:#?}"
    );
    // The basin bounds every larger volume.
    assert!(clean(&basin(Scene::wc()).check(&maximum)));
    // A fixed size needs both: the basin obstructs the smallest volume.
    let outcome = basin(Scene::wc()).check(&[
        ("size_mode", text("fixed")),
        ("size_tolerance", metres(0.05)),
    ]);
    assert_eq!(
        findings(&outcome),
        [(
            "left clearance (0.65 m wide, 0.85 m deep, 1.95 m high) is obstructed by \
             cad:model/basin"
                .to_owned(),
            vec!["basin".to_owned()]
        )],
        "{outcome:#?}"
    );
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
    // A maximum without a tolerance says nothing.
    let outcome = Scene::wc().check(&[("size_mode", text("maximum"))]);
    assert_eq!(
        unevaluated(&outcome)[0].0,
        NotEvaluatedReason::InvalidDeclaration
    );
}

#[test]
fn a_floating_maximum_is_searched_too() {
    // A larger volume slides past the basin.
    let outcome = basin(Scene::wc()).check(&[
        ("size_mode", text("maximum")),
        ("size_tolerance", metres(0.05)),
        ("slide_from", metres(0.0)),
        ("slide_to", metres(1.5)),
    ]);
    assert_eq!(findings(&outcome).len(), 3, "{outcome:#?}");
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
}

#[test]
fn a_turned_component_floats_its_volume_as_an_interval() {
    // Turned 30 degrees, the WC's faces are intervals. Its left side looks
    // south-west, so sliding the volume clear of the south wall pushes it
    // into the west wall: 0.5 m deep it fits in between, 0.9 m deep nowhere.
    let turned = || Scene::wc().turned_wc(30_f64.to_radians());
    let slide = [("slide_from", metres(0.0)), ("slide_to", metres(1.0))];
    let outcome = turned().check(&[slide[0].clone(), slide[1].clone(), ("depth", metres(0.5))]);
    assert!(clean(&outcome), "{outcome:#?}");
    let outcome = turned().check(&slide);
    let found = findings(&outcome);
    assert_eq!(found.len(), 1, "{outcome:#?}");
    assert!(found[0].0.contains("fits nowhere"), "{outcome:#?}");
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
}

/// The WC's room with its south wall (behind the WC) and west wall, and a
/// chair north of the WC, in its front area.
fn walled(scene: Scene) -> Scene {
    scene
        .body("south", "wall", cuboid([-0.2, -0.2, 0.0], [3.2, 0.0, 2.5]))
        .body("west", "wall", cuboid([-0.2, -0.2, 0.0], [0.0, 3.2, 2.5]))
        .body("chair", "fixture", cuboid([1.1, 1.2, 0.0], [1.3, 1.4, 0.9]))
}

fn front_area(front: &str) -> Vec<(&'static str, ParameterValue)> {
    let mut parameters = vec![
        ("side", text("front")),
        ("front_axis", text(front)),
        ("width", metres(0.8)),
        ("depth", metres(1.2)),
        ("align", text("centre")),
    ];
    if front == "against-wall" {
        parameters.extend([
            ("wall_selector", selector(kind("wall"))),
            ("wall_reach", metres(1.5)),
            ("wall_inset", metres(0.01)),
        ]);
    }
    parameters
}

#[test]
fn the_front_against_a_wall_does_not_depend_on_the_local_axes() {
    // The WC's placement turned a quarter: forward points west, and the
    // front area there misses the chair and runs into the walls.
    let quarter = |scene: Scene| Scene {
        right: [0.0, 1.0, 0.0],
        ..scene
    };
    let outcome = walled(quarter(Scene::wc())).check(&front_area("forward"));
    assert_eq!(findings(&outcome)[0].1, ["south", "west"], "{outcome:#?}");
    // Against the south wall, both placements face north, into the chair.
    for scene in [walled(Scene::wc()), walled(quarter(Scene::wc()))] {
        let outcome = scene.check(&front_area("against-wall"));
        assert_eq!(
            findings(&outcome),
            [(
                "front clearance (0.8 m wide, 1.2 m deep, 2 m high) is obstructed by \
                 cad:model/chair"
                    .to_owned(),
                vec!["chair".to_owned()]
            )],
            "{outcome:#?}"
        );
        let derived = &outcome.findings()[0].evidence;
        assert!(
            derived.iter().any(|evidence| !evidence.exact
                && evidence.locator
                    == "axioval:derived.front:cad:model/wc:against=cad:model/south:side=-second"),
            "{outcome:#?}"
        );
    }
}

#[test]
fn a_turned_room_derives_its_front_within_the_rectangle_error() {
    // Everything turned 20 degrees about the WC's centre: the rectangle's
    // axes are rounded, so the volume is widened by the arc they may turn.
    let turn = 20_f64.to_radians();
    let pivot = [1.2, 0.35];
    let scene = Scene::wc()
        .turned_wc(turn)
        .body(
            "south",
            "wall",
            turned([-0.2, -0.2, 0.0], [3.2, 0.0, 2.5], turn, pivot),
        )
        .body(
            "west",
            "wall",
            turned([-0.2, -0.2, 0.0], [0.0, 3.2, 2.5], turn, pivot),
        )
        .body(
            "chair",
            "fixture",
            turned([1.1, 1.2, 0.0], [1.3, 1.4, 0.9], turn, pivot),
        );
    let outcome = scene.check(&front_area("against-wall"));
    assert_eq!(findings(&outcome).len(), 1, "{outcome:#?}");
    assert_eq!(findings(&outcome)[0].1, ["chair"], "{outcome:#?}");
}

#[test]
fn a_front_between_two_walls_as_near_is_not_decided() {
    // A wall flush with the WC's west face: two sides stand against a wall.
    let scene =
        walled(Scene::wc()).body("corner", "wall", cuboid([0.8, 0.0, 0.0], [1.0, 3.0, 2.5]));
    let outcome = scene.check(&front_area("against-wall"));
    assert!(outcome.findings().is_empty(), "{outcome:#?}");
    let reasons = unevaluated(&outcome);
    assert_eq!(reasons.len(), 1, "{outcome:#?}");
    assert_eq!(reasons[0].0, NotEvaluatedReason::IncompleteEvidence);
    assert!(reasons[0].1.contains("front not decided"), "{outcome:#?}");
    // Without a wall in reach there is no front either.
    let outcome = Scene::wc().check(&front_area("against-wall"));
    let reasons = unevaluated(&outcome);
    assert!(
        reasons.len() == 1 && reasons[0].1.contains("no wall lies within 1.5 m"),
        "{outcome:#?}"
    );
    // The walls go with `against-wall` only, and it needs them.
    let mut parameters = front_area("forward");
    parameters.push(("wall_reach", metres(1.0)));
    let outcome = walled(Scene::wc()).check(&parameters);
    assert_eq!(
        unevaluated(&outcome)[0].0,
        NotEvaluatedReason::InvalidDeclaration
    );
    let outcome = walled(Scene::wc()).check(&[("front_axis", text("against-wall"))]);
    assert_eq!(
        unevaluated(&outcome)[0].0,
        NotEvaluatedReason::InvalidDeclaration
    );
}

fn texts(values: &[&str]) -> ParameterValue {
    ParameterValue::StringList {
        value: values.iter().map(|value| (*value).to_owned()).collect(),
    }
}

/// A front area as wide as the WC, clamped to 0.8 to 1.2 m, 0.5 m deep,
/// from the floor to 50 mm above the WC's top (0.45 m).
fn sized_front(scene: Scene) -> CapabilityEvaluation {
    scene.check(&[
        ("side", text("front")),
        ("align", text("centre")),
        ("width", none()),
        ("width_mode", text("component_clamped")),
        ("width_minimum", metres(0.8)),
        ("width_maximum", metres(1.2)),
        ("depth", metres(0.5)),
        ("height", none()),
        ("top_datum", text("top")),
        ("top_offset", metres(0.05)),
    ])
}

#[test]
fn a_front_area_is_sized_from_the_component_and_its_top() {
    // The WC is 0.4 m wide: clamped, the area spans x 0.8 to 1.6.
    let post = |x: f64, bottom: f64| {
        Scene::wc().body(
            "post",
            "fixture",
            cuboid([x, 0.9, bottom], [x + 0.05, 1.0, bottom + 0.3]),
        )
    };
    let outcome = sized_front(post(1.5, 0.0));
    assert_eq!(
        findings(&outcome),
        [(
            "front clearance (0.8 m wide, 0.5 m deep, up to 0.05 m above the component's top) \
             is obstructed by cad:model/post"
                .to_owned(),
            vec!["post".to_owned()]
        )],
        "{outcome:#?}"
    );
    // Beyond the clamped width, or above the top datum, it is clear.
    assert!(clean(&sized_front(post(1.65, 0.0))));
    assert!(clean(&sized_front(post(1.5, 0.5))));
    // Just below 0.45 m it still reaches in.
    assert_eq!(findings(&sized_front(post(1.5, 0.44))).len(), 1);
}

#[test]
fn a_component_sized_width_follows_the_component() {
    // `component_plus` 0.3 m: 0.7 m wide over the 0.4 m WC.
    let outcome = Scene::wc()
        .body("post", "fixture", cuboid([1.5, 0.9, 0.0], [1.55, 1.0, 0.3]))
        .check(&[
            ("side", text("front")),
            ("align", text("centre")),
            ("width", metres(0.3)),
            ("width_mode", text("component_plus")),
            ("depth", metres(0.5)),
        ]);
    assert_eq!(findings(&outcome).len(), 1, "{outcome:#?}");
    assert!(
        findings(&outcome)[0]
            .0
            .starts_with("front clearance (0.7 m wide"),
        "{outcome:#?}"
    );
    // Measured from its midline, 0.5 m deep reaches only 0.15 m past the
    // WC's front (y 0.7 to 0.85), short of the post.
    let outcome = Scene::wc()
        .body("post", "fixture", cuboid([1.1, 0.9, 0.0], [1.3, 1.0, 0.3]))
        .check(&[
            ("side", text("front")),
            ("align", text("centre")),
            ("width", metres(0.4)),
            ("depth", metres(0.5)),
            ("depth_from", text("midline")),
        ]);
    assert!(clean(&outcome), "{outcome:#?}");
}

/// A thin panel right in front of the WC (y 0.72 to 0.82) and a front area
/// 0.6 m wide and 0.5 m deep that may float away from the WC.
fn floating_front(to: f64) -> CapabilityEvaluation {
    Scene::wc()
        .body(
            "panel",
            "fixture",
            cuboid([1.1, 0.72, 0.0], [1.3, 0.82, 1.0]),
        )
        .check(&[
            ("side", text("front")),
            ("align", text("centre")),
            ("width", metres(0.6)),
            ("depth", metres(0.5)),
            ("depth_slide_from", metres(0.0)),
            ("depth_slide_to", metres(to)),
        ])
}

#[test]
fn a_volume_floats_away_from_the_component() {
    // Obstructed where it starts, free 150 mm out: a 200 mm float passes.
    assert!(clean(&floating_front(0.2)));
    let outcome = floating_front(0.1);
    assert_eq!(
        findings(&outcome),
        [(
            "front clearance (0.6 m wide, 0.5 m deep, 2 m high) fits nowhere between 0 m and \
             0.1 m away from the component in cad:model/room"
                .to_owned(),
            vec!["room".to_owned()]
        )],
        "{outcome:#?}"
    );
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
}

#[test]
fn one_free_side_is_enough_under_any() {
    let both = [
        ("side", none()),
        ("sides", texts(&["left", "right"])),
        ("align", text("centre")),
    ];
    // The basin blocks the left side only.
    let mut any = both.to_vec();
    any.push(("quantifier", text("any")));
    assert!(clean(&basin(Scene::wc()).check(&any)));
    let outcome = basin(Scene::wc()).check(&both);
    let found = findings(&outcome);
    assert_eq!(found.len(), 1, "{outcome:#?}");
    assert!(found[0].0.starts_with("left clearance"), "{outcome:#?}");
    // With a second basin on the right, neither side is free.
    let outcome = basin(Scene::wc())
        .body("basin2", "basin", cuboid([1.8, 0.2, 0.8], [2.1, 0.5, 1.0]))
        .check(&any);
    assert_eq!(
        findings(&outcome),
        [(
            "left or right clearance has no free side: left (0.7 m wide, 0.9 m deep, 2 m high) \
             is obstructed by cad:model/basin; right (0.7 m wide, 0.9 m deep, 2 m high) is \
             obstructed by cad:model/basin2"
                .to_owned(),
            vec!["basin".to_owned(), "basin2".to_owned()]
        )],
        "{outcome:#?}"
    );
}

/// A front area 0.8 m wide and 1.2 m deep (y 0.7 to 1.9) at the WC's
/// bottom, over a slab whose top lies at `top` and which ends at `edge`.
fn over_slab(edge: f64, top: f64) -> CapabilityEvaluation {
    Scene::wc()
        .body(
            "slab",
            "slab",
            cuboid([-1.0, -1.0, top - 0.2], [4.0, edge, top]),
        )
        .check(&[
            ("side", text("front")),
            ("align", text("centre")),
            ("width", metres(0.8)),
            ("depth", metres(1.2)),
            ("height_reference", text("bottom")),
            ("space_path", none()),
            ("obstacles", selector(kind("fixture"))),
            ("support_selector", selector(kind("slab"))),
            ("support_tolerance", metres(0.02)),
        ])
}

#[test]
fn a_clear_area_overhanging_the_slab_edge_is_unsupported() {
    assert!(clean(&over_slab(3.0, 0.0)));
    let outcome = over_slab(1.0, 0.0);
    assert_eq!(
        findings(&outcome),
        [(
            "front clearance (0.8 m wide, 1.2 m deep, 2 m high) is not wholly supported: part \
             of it lies over no top of the supports within 0.02 m of its base"
                .to_owned(),
            Vec::new()
        )],
        "{outcome:#?}"
    );
    // A slab 0.1 m below the base is beyond the tolerance.
    assert_eq!(findings(&over_slab(3.0, -0.1)).len(), 1);
}

#[test]
fn the_new_declarations_fail_closed() {
    let invalid = |parameters: &[(&str, ParameterValue)]| {
        let outcome = Scene::wc().check(parameters);
        assert_eq!(
            unevaluated(&outcome)[0].0,
            NotEvaluatedReason::InvalidDeclaration,
            "{outcome:#?}"
        );
    };
    // Both `side` and `sides`, or `any` with one side.
    invalid(&[("sides", texts(&["left"]))]);
    invalid(&[("quantifier", text("any"))]);
    // A height and a top datum.
    invalid(&[("top_datum", text("top"))]);
    // A clamp without its bounds, or bounds without the clamp.
    invalid(&[("width_mode", text("component_clamped"))]);
    invalid(&[("width_minimum", metres(0.5))]);
    // A support for a floating volume, or without a tolerance.
    invalid(&[
        ("support_selector", selector(kind("slab"))),
        ("support_tolerance", metres(0.02)),
        ("depth_slide_from", metres(0.0)),
        ("depth_slide_to", metres(0.1)),
    ]);
    invalid(&[("support_selector", selector(kind("slab")))]);
    // A depth float back into the component.
    invalid(&[
        ("depth_slide_from", metres(-0.1)),
        ("depth_slide_to", metres(0.1)),
    ]);
}

/// The WC stands in for a door 0.7 m deep (y 0 to 0.7) whose clear width
/// it states; a cupboard stands 1.25 m in front of it (y 1.95 to 2.2).
fn door_with_width(width: PropertyValue) -> CapabilityEvaluation {
    let mut scene = Scene::wc().body(
        "cupboard",
        "cupboard",
        cuboid([1.0, 1.95, 0.0], [1.4, 2.2, 1.0]),
    );
    scene.clear_width = Some(width);
    scene.check(&[
        ("side", text("front")),
        ("align", text("centre")),
        ("width", metres(0.4)),
        ("depth", metres(1.5)),
        ("depth_mode", text("less_clear_width")),
        ("depth_minimum", metres(1.2)),
        (
            "clear_width_property",
            ParameterValue::PropertyReference {
                property: "ClearWidth".into(),
                property_set: Some("Door".into()),
            },
        ),
    ])
}

fn width(value: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value,
        dimension: QuantityDimension::Length,
    }
}

fn width_between(lower: f64, upper: f64) -> PropertyValue {
    PropertyValue::Measured {
        lower,
        upper,
        dimension: Some(QuantityDimension::Length),
    }
}

/// The area in front of a door is `max(1.2 m, 1.5 m - clear width)` deep:
/// a 0.9 m door needs 1.2 m, clear of the cupboard; a 0.2 m door needs
/// 1.3 m, which the cupboard obstructs.
#[test]
fn the_depth_in_front_of_a_door_shrinks_as_the_door_widens() {
    assert!(clean(&door_with_width(width(0.9))));
    let narrow = door_with_width(width(0.2));
    let found = findings(&narrow);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].0.contains("1.3 m deep") && found[0].0.contains("cupboard"),
        "{found:?}"
    );
    assert!(
        found[0].0.ends_with(
            "; the depth is 1.5 m less the clear width (Door.ClearWidth) 0.2 m, at least 1.2 m"
        ),
        "{found:?}"
    );
    assert!(
        narrow.findings()[0]
            .evidence
            .iter()
            .any(|evidence| evidence.locator == "Door.ClearWidth")
    );
}

/// A clear width known only as an interval gives a depth interval: decided
/// where every depth agrees, not evaluated where the verdict depends on it.
#[test]
fn a_clear_width_interval_decides_only_where_both_depths_agree() {
    // 0.85 to 0.9 m: depths of 1.2 to 1.25 m, all clear.
    assert!(clean(&door_with_width(width_between(0.85, 0.9))));
    // 0.1 to 0.2 m: depths of 1.3 to 1.4 m, all obstructed.
    assert_eq!(findings(&door_with_width(width_between(0.1, 0.2))).len(), 1);
    // 0.2 to 0.9 m: 1.2 m is clear, 1.3 m is not.
    let open = door_with_width(width_between(0.2, 0.9));
    assert!(open.findings().is_empty());
    let undecided = unevaluated(&open);
    assert_eq!(undecided.len(), 1, "{undecided:?}");
    assert!(
        undecided[0]
            .1
            .contains("the clear width (Door.ClearWidth) between 0.2 m and 0.9 m"),
        "{undecided:?}"
    );
}

/// A depth from the clear width needs a source for it, an unreadable
/// stated width is not evaluated, and the sources are refused elsewhere.
#[test]
fn a_depth_from_the_clear_width_fails_closed() {
    let unreadable = door_with_width(PropertyValue::String("wide".into()));
    assert_eq!(unevaluated(&unreadable).len(), 1);
    assert!(
        unevaluated(&unreadable)[0]
            .1
            .contains("needs the clear width")
    );
    let mut scene = Scene::wc();
    scene.clear_width = Some(width(0.9));
    let missing = scene.check(&[
        ("side", text("front")),
        ("depth", metres(1.5)),
        ("depth_mode", text("less_clear_width")),
    ]);
    assert_eq!(
        unevaluated(&missing)[0].0,
        NotEvaluatedReason::InvalidDeclaration
    );
    let stray = Scene::wc().check(&[(
        "clear_width_property",
        ParameterValue::PropertyReference {
            property: "ClearWidth".into(),
            property_set: None,
        },
    )]);
    assert_eq!(
        unevaluated(&stray)[0].0,
        NotEvaluatedReason::InvalidDeclaration
    );
}
