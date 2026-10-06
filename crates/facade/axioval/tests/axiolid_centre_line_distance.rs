//! Centre-line distances from fixtures to the walls beside them, measured
//! on real meshes.
#![cfg(feature = "axiolid")]
#![allow(missing_docs)]

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval::axiolid::{AxiolidGeometry, AxiolidPlanSpanService};
use axioval::engine::{
    CapabilityEvaluation, CompiledRule, PlanSpanServiceHandle, PropertyRequest, PropertyResolution,
    PropertyResolutionError, PropertyResolutionService, PropertyResolutionServiceHandle,
    RuleCapability, RuleContext, ServiceRegistry,
};
use axioval::ir::contract::{ParameterValue, Selector, Severity};
use axioval::ir::{NotEvaluatedReason, Object, ObjectId, Project, RuleId, SourceId};
use axioval::rules::CentreLineDistance;

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
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7,
            3, 1, 2, 6, 1, 6, 5,
        ],
    )
}

/// A 0.4 m wide, 0.7 m deep WC against the south wall, its axis `axis`
/// metres east of the west wall's face, and walls `extra` besides.
struct Scene {
    objects: Vec<Object>,
    geometry: AxiolidGeometry,
}

impl Scene {
    fn wc(axis: f64) -> Self {
        Self {
            objects: Vec::new(),
            geometry: AxiolidGeometry::new(),
        }
        .body(
            "wc",
            "wc",
            cuboid([axis - 0.2, 0.0, 0.0], [axis + 0.2, 0.7, 0.4]),
        )
        .body("south", "wall", cuboid([-0.2, -0.2, 0.0], [3.2, 0.0, 2.5]))
        .body("west", "wall", cuboid([-0.2, -0.2, 0.0], [0.0, 3.2, 2.5]))
    }

    fn body(mut self, local: &str, kind: &str, mesh: TriMesh) -> Self {
        self.objects.push(Object::new(id(local), kind));
        self.geometry = self.geometry.with_mesh(id(local), mesh);
        self
    }

    fn check(self, parameters: &[(&str, ParameterValue)]) -> CapabilityEvaluation {
        let mut bound = std::collections::BTreeMap::from([
            ("wall_selector".to_owned(), selector(kind("wall"))),
            ("centre_line".to_owned(), text("against-wall")),
            ("sides".to_owned(), text("nearest")),
            ("minimum".to_owned(), metres(0.405)),
            ("maximum".to_owned(), metres(0.455)),
            ("reach".to_owned(), metres(1.0)),
            ("inset".to_owned(), metres(0.01)),
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
            id: RuleId::new("wc-axis").unwrap(),
            capability: "axioval:capability.centre-line-distance".into(),
            severity: Severity::Error,
            selector: kind("wc"),
            parameters: bound,
        };
        let project = Project::new(self.objects).unwrap();
        let mut services = ServiceRegistry::new();
        services
            .register(PlanSpanServiceHandle::new(Arc::new(
                AxiolidPlanSpanService::new(self.geometry, source()),
            )))
            .unwrap();
        // The template reads the measured sides as a run does.
        let registry =
            axioval::rules::register_builtins(axioval::engine::CapabilityRegistry::new()).unwrap();
        registry.install_measured(&mut services, &project);
        let context = RuleContext {
            project: &project,
            services: &services,
        };
        let template = CentreLineDistance.evaluate(&context, &rule);
        // Held to the implementation it replaced, on the same scene.
        let reference = axioval_rules::reference::CentreLineDistance.evaluate(&context, &rule);
        let parity = axioval::rules::parity::Parity::contract().compare(
            (
                "centre-line-distance",
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

/// Removes a default parameter.
fn none() -> ParameterValue {
    ParameterValue::StringList { value: Vec::new() }
}

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

#[test]
fn a_wc_380_mm_from_its_wall_is_too_close() {
    let outcome = Scene::wc(0.38).check(&[]);
    assert_eq!(
        findings(&outcome),
        [(
            "centre line to the nearest wall: too close: 0.38 m from cad:model/west, less than \
             the minimum 0.405 m"
                .to_owned(),
            vec!["west".to_owned()]
        )],
        "{outcome:#?}"
    );
    assert!(
        outcome.findings()[0]
            .evidence
            .iter()
            .all(|evidence| !evidence.locator.starts_with("plan-side") || !evidence.exact),
        "{outcome:#?}"
    );
}

#[test]
fn a_wc_within_the_range_passes_and_one_beyond_it_is_too_far() {
    assert!(clean(&Scene::wc(0.43).check(&[])));
    let outcome = Scene::wc(0.6).check(&[]);
    assert_eq!(
        findings(&outcome),
        [(
            "centre line to the nearest wall: too far: 0.6 m from cad:model/west, more than the \
             maximum 0.455 m"
                .to_owned(),
            vec!["west".to_owned()]
        )],
        "{outcome:#?}"
    );
    // Along its long axis the centre line runs the same way.
    assert!(clean(
        &Scene::wc(0.43).check(&[("centre_line", text("long"))])
    ));
}

#[test]
fn a_missing_second_wall_is_reported_under_both() {
    let outcome = Scene::wc(0.43).check(&[("sides", text("both"))]);
    assert_eq!(
        findings(&outcome),
        [(
            "centre line to the right: no wall nearby (none within 1 m)".to_owned(),
            Vec::new()
        )],
        "{outcome:#?}"
    );
    // A second wall 0.43 m east of the axis closes the stall.
    let outcome = Scene::wc(0.43)
        .body("east", "wall", cuboid([0.86, -0.2, 0.0], [1.06, 3.2, 2.5]))
        .check(&[("sides", text("both"))]);
    assert!(clean(&outcome), "{outcome:#?}");
    // Without any wall in reach, `nearest` finds none either.
    let outcome = Scene::wc(0.43).check(&[
        ("centre_line", text("long")),
        ("wall_selector", selector(kind("partition"))),
    ]);
    assert_eq!(
        findings(&outcome),
        [(
            "centre line to the nearest wall: no wall nearby (none within 1 m)".to_owned(),
            Vec::new()
        )],
        "{outcome:#?}"
    );
}

#[test]
fn declarations_fail_closed() {
    for parameters in [
        vec![("reach", metres(0.4))],
        vec![("minimum", none()), ("maximum", none())],
        vec![("minimum", metres(0.5))],
        vec![("sides", text("left"))],
        vec![("centre_line", text("forward"))],
    ] {
        let outcome = Scene::wc(0.43).check(&parameters);
        assert_eq!(
            unevaluated(&outcome)[0].0,
            NotEvaluatedReason::InvalidDeclaration,
            "{parameters:?}: {outcome:#?}"
        );
    }
    // A WC in the corner touches two walls: its back is not decided.
    let outcome = Scene::wc(0.2).check(&[]);
    let reasons = unevaluated(&outcome);
    assert!(
        reasons.len() == 1 && reasons[0].1.contains("front not decided"),
        "{outcome:#?}"
    );
}

/// Answers the measured set as a run does; nothing else.
struct Measured {
    services: ServiceRegistry,
    project: Project,
}

impl PropertyResolutionService for Measured {
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        axioval::engine::measured_value(
            &self.services,
            &self.project,
            request.object_id(),
            request.property(),
        )
    }

    fn enumerate(
        &self,
        _: &axioval::engine::PropertyEnumerationRequest,
    ) -> Result<axioval::engine::PropertyEnumeration, PropertyResolutionError> {
        Err(PropertyResolutionError::InvalidRequest)
    }
}

impl Scene {
    /// `requirement` as an expression rule over the same scene, with the
    /// built-in measured values installed.
    fn express(self, requirement: &serde_json::Value) -> CapabilityEvaluation {
        let rule = CompiledRule {
            id: RuleId::new("wc-axis").unwrap(),
            capability: "axioval:capability.expression".into(),
            severity: Severity::Error,
            selector: kind("wc"),
            parameters: std::collections::BTreeMap::from([(
                "requirement".to_owned(),
                ParameterValue::Expression {
                    value: serde_json::from_value(requirement.clone()).unwrap(),
                },
            )]),
        };
        let project = Project::new(self.objects).unwrap();
        let registry =
            axioval::rules::register_builtins(axioval::engine::CapabilityRegistry::new()).unwrap();
        let spans = PlanSpanServiceHandle::new(Arc::new(AxiolidPlanSpanService::new(
            self.geometry,
            source(),
        )));
        let mut measuring = ServiceRegistry::new();
        measuring.register(spans.clone()).unwrap();
        registry.install_measured(&mut measuring, &project);
        let mut services = ServiceRegistry::new();
        services.register(spans).unwrap();
        services
            .register(PropertyResolutionServiceHandle::new(Arc::new(Measured {
                services: measuring,
                project: project.clone(),
            })))
            .unwrap();
        axioval::rules::ExpressionRequirement.evaluate(
            &RuleContext {
                project: &project,
                services: &services,
            },
            &rule,
        )
    }
}

/// The centre-line distance as a measured value reaches the capability's
/// verdicts: the nearer side for `nearest`, and for `both` the nearer side
/// against the minimum and the farther against the maximum. Held to the
/// parity harness, which differs only in the exactness of a missing wall's
/// finding.
#[test]
#[allow(clippy::type_complexity, clippy::too_many_lines)]
fn the_centre_line_distance_as_a_value_reaches_the_verdicts() {
    use serde_json::{Value, json};
    let distance = |line: &str, side: &str, walls: &str| {
        let name = format!(
            "centre_line_distance;walls={walls};centre_line={line};side={side};reach=1;inset=0.01"
        );
        json!({"kind": "round", "operand": {"kind": "property", "propertySet": "axioval:measured",
            "property": name}, "step": {"kind": "literal", "value": {"type": "quantity",
            "value": 1e-6, "unit": "m"}}})
    };
    let m = |value: f64| json!({"kind": "literal", "value": {"type": "quantity", "value": value, "unit": "m"}});
    let within = |line: &str, both: bool, walls: &str| -> Value {
        let near = distance(line, "nearest", walls);
        let far = distance(line, if both { "farther" } else { "nearest" }, walls);
        json!({"kind": "and", "operands": [
            {"kind": "isDefined", "operand": far.clone()},
            {"kind": "compare", "operator": "greaterThanOrEquals", "left": near, "right": m(0.405)},
            {"kind": "compare", "operator": "lessThanOrEquals", "left": far, "right": m(0.455)}]})
    };
    let east = || cuboid([0.86, -0.2, 0.0], [1.06, 3.2, 2.5]);
    let cases: Vec<(Box<dyn Fn() -> Scene>, Vec<(&str, ParameterValue)>, Value)> = vec![
        (
            Box::new(|| Scene::wc(0.38)),
            vec![],
            within("against-wall", false, "wall"),
        ),
        (
            Box::new(|| Scene::wc(0.43)),
            vec![],
            within("against-wall", false, "wall"),
        ),
        (
            Box::new(|| Scene::wc(0.6)),
            vec![],
            within("against-wall", false, "wall"),
        ),
        (
            Box::new(|| Scene::wc(0.43)),
            vec![("centre_line", text("long"))],
            within("long", false, "wall"),
        ),
        (
            Box::new(|| Scene::wc(0.43)),
            vec![("sides", text("both"))],
            within("against-wall", true, "wall"),
        ),
        (
            Box::new(move || Scene::wc(0.43).body("east", "wall", east())),
            vec![("sides", text("both"))],
            within("against-wall", true, "wall"),
        ),
        (
            Box::new(|| Scene::wc(0.43)),
            vec![
                ("centre_line", text("long")),
                ("wall_selector", selector(kind("partition"))),
            ],
            within("long", false, "partition"),
        ),
        (
            Box::new(|| Scene::wc(0.2)),
            vec![],
            within("against-wall", false, "wall"),
        ),
    ];
    for (index, (scene, parameters, requirement)) in cases.into_iter().enumerate() {
        let expected = scene().check(&parameters);
        let outcome = scene().express(&requirement);
        let parity = axioval::rules::parity::compare_evaluations(
            ("centre-line-distance", &expected),
            ("expression", &outcome),
        );
        if index == 4 || index == 6 {
            // No wall within reach (on one side): the capability cites the
            // inexact side measurements, the expression the exact absence
            // of a distance within reach.
            assert_eq!(
                parity.differences,
                vec![axioval::rules::parity::Difference {
                    scope: id("wc").into(),
                    capability: Some(axioval::rules::parity::Outcome::Finding {
                        severity: axioval::ir::Severity::Error,
                        exact: false,
                    }),
                    expression: Some(axioval::rules::parity::Outcome::Finding {
                        severity: axioval::ir::Severity::Error,
                        exact: true,
                    }),
                    details: vec![],
                }],
                "case {index}"
            );
        } else {
            assert!(parity.holds(), "case {index}:\n{}", parity.diff());
        }
    }
}

/// Each refused declaration is worded as the capability worded it.
#[test]
fn refusals_keep_their_words() {
    let refused = |parameters: &[(&str, ParameterValue)]| {
        unevaluated(&Scene::wc(0.43).check(parameters))
            .into_iter()
            .map(|(_, message)| message)
            .collect::<Vec<_>>()
    };
    let area = ParameterValue::Quantity {
        value: 1.0,
        unit: "m2".into(),
    };
    for (parameters, message) in [
        (
            vec![("centre_line", text("forward"))],
            "centre_line `forward` is unsupported; use `long`, `short` or `against-wall`",
        ),
        (
            vec![("sides", text("left"))],
            "sides `left` is unsupported; use `nearest` or `both`",
        ),
        (
            vec![("minimum", metres(-0.1))],
            "`minimum` must be a finite length, not negative",
        ),
        (vec![("maximum", area.clone())], "`maximum` is not a length"),
        (
            vec![
                ("reach", metres(0.0)),
                ("minimum", none()),
                ("maximum", none()),
            ],
            "`reach` is required and must be positive",
        ),
        (
            vec![("minimum", none()), ("maximum", none())],
            "declare `minimum`, `maximum` or both",
        ),
        (
            vec![("minimum", metres(0.5))],
            "`minimum` exceeds `maximum`",
        ),
        (
            vec![("reach", metres(0.4))],
            "`reach` must be at least `minimum` and `maximum`: a wall beyond it is none",
        ),
        (vec![("inset", area)], "`inset` is not a length"),
    ] {
        assert_eq!(
            refused(&parameters),
            vec![format!("centre-line-distance: {message}")],
            "{parameters:?}"
        );
    }
}

/// Generated stalls: WCs at many distances from their walls, with or
/// without a second wall at many distances, judged along every centre
/// line, on one side or both, under many bounds; each evaluation is held
/// to the implementation the template replaced.
#[test]
fn generated_stalls_hold_parity() {
    let mut judged = 0;
    for step in 0..12_u32 {
        let axis = 0.2 + f64::from(step) * 0.07;
        for east in [None, Some(0.75), Some(1.1), Some(1.6)] {
            for (line, sides) in [
                ("long", "both"),
                ("short", "nearest"),
                ("against-wall", "nearest"),
                ("against-wall", "both"),
            ] {
                for (minimum, maximum) in [
                    (Some(0.405), Some(0.455)),
                    (Some(0.3), None),
                    (None, Some(0.6)),
                    (Some(0.1), Some(0.9)),
                ] {
                    let mut scene = Scene::wc(axis);
                    if let Some(east) = east {
                        scene = scene.body(
                            "east",
                            "wall",
                            cuboid([axis + east, -0.2, 0.0], [axis + east + 0.2, 3.2, 2.5]),
                        );
                    }
                    let bound = |value: Option<f64>| value.map_or_else(none, metres);
                    // `check` holds the template to the reference.
                    let outcome = scene.check(&[
                        ("centre_line", text(line)),
                        ("sides", text(sides)),
                        ("minimum", bound(minimum)),
                        ("maximum", bound(maximum)),
                    ]);
                    judged += outcome.findings().len() + outcome.not_evaluated_outcomes().len();
                }
            }
        }
    }
    assert!(judged > 0);
}
