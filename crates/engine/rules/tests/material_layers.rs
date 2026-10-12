//! The material layers of building elements (`layers`, #334): ordered
//! inside first from the spaces each element bounds, and folded into a
//! U-value with the thermal conductivities of a catalogue.
#![allow(missing_docs, clippy::needless_pass_by_value)]

mod common;

use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, CompleteRelationshipSelection, Derivation, DerivedRelationshipService,
    DerivedRelationshipServiceHandle, ExecutionPlan, LayerAxis, LayerDirection, LayerSense,
    MaterialLayer, MaterialLayerError, MaterialLayerService, MaterialLayerServiceHandle,
    MaterialLayers, MeasuredMember, Measurement, MemberValue, MetricDirection, MetricFrame,
    MetricPoint, ObjectFrame, ObjectFrameError, ObjectFrameService, ObjectFrameServiceHandle,
    ObjectFront, RelationshipSelectionError, RelationshipSelectionRequest, SourceSnapshot,
    SuppliedCatalogue, compile, measured_members,
};
use axioval_ir::contract::ParameterDefinition;
use axioval_ir::{DefinitionPackage, Evidence, NotEvaluatedReason, ObjectId, Report};
use axioval_rules::register_builtins;
use common::runtime::{definition, definitions, entity, rule, ruleset, run, session, snapshot};
use common::{Model, id, source};
use serde_json::{Value, json};

const EXPRESSION: &str = "axioval:capability.expression";
const LAYERS: &str = "layers;spaces=space";

/// Invented materials and their thermal conductivities, W/(m·K).
const MATERIALS: &str = "name,lambda\n\
                         ex-lime-plaster,0.70\n\
                         ex-clay-brick,0.81\n\
                         ex-foam-board,0.035\n";

/// A layer of `material` `thickness` metres thick.
fn layer(locator: &str, material: &str, thickness: f64) -> MaterialLayer {
    MaterialLayer {
        locator: locator.into(),
        material: Some(material.into()),
        name: None,
        category: None,
        thickness: Ok(thickness),
        ventilated: None,
        priority: None,
    }
}

/// The same wall build-up, outside first: foam board, clay brick, lime
/// plaster.
fn outside_first() -> Vec<MaterialLayer> {
    vec![
        layer("#11", "ex-foam-board", 0.14),
        layer("#12", "ex-clay-brick", 0.175),
        layer("#13", "ex-lime-plaster", 0.015),
    ]
}

/// The layers each wall states, all stacking along its frame's second
/// axis (world +y):
///
/// - `w1` outside first, its room on its `+` face (+y): the last layer is
///   inside;
/// - `w2` inside first, its room on its `-` face (-y): the first is inside;
/// - `w3` a partition with rooms on both faces;
/// - `w4` as `w1`, but stating no direction;
/// - `w5` as `w1`, its brick a material no catalogue lists;
/// - `w6` a constituent set, refused by name.
struct Layers(Vec<SourceSnapshot>);

impl MaterialLayerService for Layers {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.0
    }

    fn material_layers(&self, object: &ObjectId) -> Result<MaterialLayers, MaterialLayerError> {
        let along = Some(LayerDirection {
            axis: LayerAxis::Second,
            sense: LayerSense::Positive,
        });
        let (layers, direction) = match object.local_id.as_str() {
            "w1" | "w3" => (outside_first(), along),
            "w2" => (outside_first().into_iter().rev().collect(), along),
            "w4" => (outside_first(), None),
            "w5" => {
                let mut layers = outside_first();
                layers[1].material = Some("ex-mystery-block".into());
                (layers, along)
            }
            "w6" => {
                return Err(MaterialLayerError::NotLayered {
                    object: object.clone(),
                    kind: "constituent-set".into(),
                });
            }
            _ => return Err(MaterialLayerError::UnknownObject(object.clone())),
        };
        MaterialLayers::try_new(
            object.clone(),
            layers,
            direction,
            Evidence::exact(source(), format!("layers:{}", object.local_id)),
        )
    }
}

/// Every wall placed square to the world axes.
struct Frames(Vec<SourceSnapshot>);

impl ObjectFrameService for Frames {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.0
    }

    fn object_frame(&self, object: &ObjectId) -> Result<ObjectFrame, ObjectFrameError> {
        let [right, forward, up] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
            .map(|axis| MetricDirection::try_new(axis).unwrap());
        ObjectFrame::try_new(
            object.clone(),
            MetricFrame::try_new(
                MetricPoint::try_new(object.clone(), [0.0; 3]).unwrap(),
                right,
                forward,
                up,
            )
            .unwrap(),
            ObjectFront::NotStated,
            Evidence::exact(source(), format!("placement:{}", object.local_id)),
        )
    }
}

/// The rooms beside each wall's faces, as geometry derives them: `r1`
/// beside `w1`'s, `w4`'s and `w5`'s `+` face, `r2` beside `w2`'s `-`
/// face, both beside `w3`'s two faces.
struct Beside;

impl DerivedRelationshipService for Beside {
    fn derive(
        &self,
        derivation: &Derivation,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        let Derivation::AdjacentAcross { .. } = derivation else {
            return Err(RelationshipSelectionError::InvalidRequest);
        };
        let wall = request.anchor();
        let faces: &[(&str, char)] = match wall.local_id.as_str() {
            "w1" | "w4" | "w5" => &[("r1", '+')],
            "w2" => &[("r2", '-')],
            "w3" => &[("r1", '+'), ("r2", '-')],
            _ => &[],
        };
        let mut candidates = Vec::new();
        let mut evidence = vec![Evidence::exact(source(), format!("{derivation}:{wall}"))];
        for (room, side) in faces {
            candidates.push(id(room));
            evidence.push(Evidence::exact(
                source(),
                format!(
                    "{derivation}:{wall}->{}:side={side}(0.000000,1.000000,0.000000):overlap=4",
                    id(room)
                ),
            ));
        }
        CompleteRelationshipSelection::try_new(request.clone(), candidates, evidence)
    }
}

fn model() -> Model {
    ["w1", "w2", "w3", "w4", "w5", "w6"]
        .into_iter()
        .fold(Model::default(), |model, wall| model.object(wall, "wall"))
        .object("r1", "space")
        .object("r2", "space")
}

/// The walls' layers, listed as a rule reads them.
fn listed(wall: &str, list: &str) -> Vec<MeasuredMember> {
    let (project, mut services) = model().services();
    services
        .register(MaterialLayerServiceHandle::new(Arc::new(Layers(vec![
            snapshot(),
        ]))))
        .unwrap();
    services
        .register(ObjectFrameServiceHandle::new(Arc::new(Frames(vec![
            snapshot(),
        ]))))
        .unwrap();
    services
        .register(DerivedRelationshipServiceHandle::new(Arc::new(Beside)))
        .unwrap();
    register_builtins(CapabilityRegistry::new())
        .unwrap()
        .install_measured(&mut services, &project);
    measured_members(&services, &id(wall), list).unwrap()
}

fn text(member: &MeasuredMember, field: &str) -> String {
    match &member.fields[field] {
        MemberValue::Text { text } => text.clone(),
        other => panic!("{field} is {other:?}"),
    }
}

/// A number field's interval.
fn number(member: &MeasuredMember, field: &str) -> (f64, f64) {
    match &member.fields[field] {
        MemberValue::Measured(
            Measurement::Rounded { lower, upper, .. } | Measurement::Cited { lower, upper, .. },
        ) => (*lower, *upper),
        other => panic!("{field} is {other:?}"),
    }
}

fn undecided(member: &MeasuredMember, field: &str) -> String {
    match &member.fields[field] {
        MemberValue::Undecided { why } => why.clone(),
        other => panic!("{field} is {other:?}"),
    }
}

fn materials(members: &[MeasuredMember]) -> Vec<String> {
    members
        .iter()
        .map(|member| text(member, "material"))
        .collect()
}

/// #334: a three-layer wall yields its layers inside first, whichever
/// order its source states them in, with their place and distance from
/// the inside.
#[test]
fn a_three_layer_wall_lists_its_layers_inside_first() {
    for wall in ["w1", "w2"] {
        let members = listed(wall, LAYERS);
        assert_eq!(
            materials(&members),
            ["ex-lime-plaster", "ex-clay-brick", "ex-foam-board"],
            "{wall}"
        );
        for (place, member) in members.iter().enumerate() {
            assert!(member.exact);
            #[allow(clippy::cast_precision_loss)]
            let rank = (place + 1) as f64;
            assert_eq!(number(member, "position"), (rank, rank));
        }
        let [plaster, brick, foam] = members.as_slice() else {
            unreachable!()
        };
        assert_eq!(number(plaster, "from_inside"), (0.0, 0.0));
        let (lower, upper) = number(brick, "from_inside");
        assert!(lower <= 0.015 && upper >= 0.015 && upper - lower < 1e-12);
        let (lower, upper) = number(foam, "from_inside");
        assert!(lower <= 0.19 && upper >= 0.19 && upper - lower < 1e-12);
        assert_eq!(number(foam, "thickness"), (0.14, 0.14));
        assert_eq!(text(plaster, "layer"), "layer #13 (ex-lime-plaster)");
    }
    // The source's own order stays readable: w1 states the foam first.
    let members = listed("w1", LAYERS);
    assert_eq!(number(&members[2], "index"), (1.0, 1.0));
    assert_eq!(number(&members[0], "index"), (3.0, 3.0));
}

/// Where the inside cannot be decided the layers are listed in the
/// source's order, every stated field readable and each place from the
/// inside undecided with the reason; never ordered by a guess.
#[test]
fn an_undecided_inside_leaves_only_the_order_open() {
    for (wall, list, reason) in [
        ("w3", LAYERS, "beside both of its faces"),
        ("w4", LAYERS, "states no direction"),
        ("w1", "layers", "no space kinds are given"),
        ("w1", "layers;spaces=door", "no object of the kinds `door`"),
    ] {
        let members = listed(wall, list);
        assert_eq!(
            materials(&members),
            ["ex-foam-board", "ex-clay-brick", "ex-lime-plaster"],
            "{wall} {list}"
        );
        for member in &members {
            for field in ["position", "from_inside"] {
                let why = undecided(member, field);
                assert!(why.contains(reason), "{wall} {list}: {why}");
            }
            let _ = number(member, "thickness");
        }
    }
}

fn text_value(value: &str) -> Value {
    json!({ "default": value, "translations": {} })
}

fn member(field: &str) -> Value {
    json!({"kind": "property", "propertySet": "axioval:member", "property": field})
}

fn parameter(name: &str) -> Value {
    json!({"kind": "parameter", "name": name})
}

/// U = 1 / (Rsi + Σ d/λ + Rse), λ joined from the catalogue by each
/// layer's material.
fn u_value() -> Value {
    let resistances = json!({"kind": "aggregate", "function": "sum",
        "over": {"kind": "measured", "name": LAYERS},
        "value": {"kind": "divide", "left": member("thickness"),
                  "right": {"kind": "catalogue", "catalogue": "materials", "column": "lambda",
                            "key": member("material")}}});
    json!({"kind": "divide",
        "left": {"kind": "literal", "value": {"type": "number", "value": 1.0}},
        "right": {"kind": "add",
                  "left": {"kind": "add", "left": parameter("rsi"), "right": resistances},
                  "right": parameter("rse")},
        "label": "U"})
}

fn vocabulary(registry: &CapabilityRegistry) -> DefinitionPackage {
    let mut package = definitions(registry, &[EXPRESSION], &["wall"], &[], &[]);
    let expression = package
        .definitions
        .get_mut(&definition(EXPRESSION))
        .unwrap();
    for id in ["rsi", "rse", "maximum"] {
        let parameter: ParameterDefinition = serde_json::from_value(json!({
            "id": id, "name": text_value(id), "kind": "quantity", "required": true,
        }))
        .unwrap();
        expression.parameters.insert(id.into(), parameter);
    }
    package
}

/// One rule: every wall's U-value at most `maximum` W/(m²·K), with the
/// surface resistances Rsi = 0.13 and Rse = 0.04 m²·K/W.
fn plan(maximum: f64) -> ExecutionPlan {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let requirement = json!({"kind": "compare", "operator": "lessThanOrEquals",
        "left": u_value(), "right": parameter("maximum")});
    let resistance = |value: f64| json!({"type": "quantity", "value": value, "unit": "m²·K/W"});
    let mut set = ruleset(vec![rule(
        "u",
        EXPRESSION,
        "error",
        entity("wall"),
        json!({
            "requirement": {"type": "expression", "value": requirement},
            "rsi": resistance(0.13),
            "rse": resistance(0.04),
            "maximum": {"type": "quantity", "value": maximum, "unit": "W/m²·K"},
        }),
        json!({}),
    )]);
    set.catalogues = serde_json::from_value(json!({"materials": {
        "id": "materials",
        "name": text_value("Materials"),
        "key": "name",
        "columns": [
            {"id": "name", "kind": "string"},
            {"id": "lambda", "kind": "quantity", "unit": "W/m·K"},
        ],
    }}))
    .unwrap();
    let mut plan = compile(&registry, &[vocabulary(&registry)], &set).unwrap();
    let definition = plan.catalogues()[0].clone();
    let catalogue = SuppliedCatalogue::read(
        &definition,
        MATERIALS.as_bytes(),
        "materials.csv",
        None,
        Some("2026-1"),
    )
    .unwrap();
    plan.supply_catalogue("materials", catalogue).unwrap();
    plan
}

fn check(maximum: f64) -> Report {
    let session = session(model())
        .with_service(MaterialLayerServiceHandle::new(Arc::new(Layers(vec![
            snapshot(),
        ]))))
        .unwrap()
        .with_service(ObjectFrameServiceHandle::new(Arc::new(Frames(vec![
            snapshot(),
        ]))))
        .unwrap()
        .with_derived_relationships(
            DerivedRelationshipServiceHandle::new(Arc::new(Beside)),
            &[snapshot()],
        )
        .unwrap();
    run(
        register_builtins(CapabilityRegistry::new()).unwrap(),
        plan(maximum),
        &session,
        |runtime| runtime,
    )
    .unwrap()
}

fn walls(report: &Report) -> Vec<String> {
    let mut walls: Vec<String> = report.findings().iter().map(common::subject).collect();
    walls.sort();
    walls
}

fn open(report: &Report) -> Vec<(String, NotEvaluatedReason, String)> {
    let mut open: Vec<_> = report
        .not_evaluated
        .iter()
        .map(|outcome| {
            (
                outcome
                    .object_id()
                    .map(|object| object.local_id.clone())
                    .unwrap_or_default(),
                outcome.reason.clone(),
                outcome.message.clone(),
            )
        })
        .collect();
    open.sort_by(|a, b| a.0.cmp(&b.0));
    open
}

/// The hand calculation: Rsi + 0.015/0.70 + 0.175/0.81 + 0.14/0.035 + Rse.
fn hand_u_value() -> f64 {
    1.0 / (0.13 + 0.015 / 0.70 + 0.175 / 0.81 + 0.14 / 0.035 + 0.04)
}

/// #334: the U-value folded from the layers and the catalogue's λ
/// matches the hand calculation, 0.22689 W/(m²·K): a limit just below it
/// finds every wall it can read, one just above passes them; the order
/// does not enter it, so the partition and the wall without a direction
/// are judged too. A layer whose material has no λ leaves its wall not
/// evaluated, naming the material, and a material that is no layer set is
/// not evaluated by name.
#[test]
fn the_u_value_from_catalogue_lambdas_matches_a_hand_calculation() {
    let u = hand_u_value();
    assert!((u - 0.226_888).abs() < 1e-6, "{u}");
    let below = check(u - 1e-6);
    assert_eq!(walls(&below), ["w1", "w2", "w3", "w4"], "{below:#?}");
    let above = check(u + 1e-6);
    assert!(above.findings().is_empty(), "{above:#?}");
    for report in [&below, &above] {
        let open = open(report);
        let [(mystery, reason, message), (constituents, _, refused)] = open.as_slice() else {
            panic!("{open:#?}")
        };
        assert_eq!(mystery, "w5");
        assert_eq!(*reason, NotEvaluatedReason::IncompleteEvidence, "{message}");
        assert!(
            message.contains("has no entry whose `name` is `ex-mystery-block`"),
            "{message}"
        );
        assert_eq!(constituents, "w6");
        assert!(
            refused.contains("is a constituent-set, not a layer set"),
            "{refused}"
        );
    }
    // The finding names the catalogue's statements it read.
    let finding = below
        .findings()
        .iter()
        .find(|finding| common::subject(finding) == "w1")
        .unwrap();
    assert!(
        finding
            .evidence
            .iter()
            .any(|item| item.locator.contains("#entry=ex-foam-board")),
        "{finding:#?}"
    );
}

/// A material-layer service answering with evidence that is not exact is
/// refused before its layers reach a value: layers are stated, never
/// estimated.
#[test]
fn layers_cited_approximate_are_refused() {
    let mut approximate = Evidence::exact(source(), "layers:w1");
    approximate.exact = false;
    assert_eq!(
        MaterialLayers::try_new(id("w1"), outside_first(), None, approximate),
        Err(MaterialLayerError::InexactEvidence)
    );
}
