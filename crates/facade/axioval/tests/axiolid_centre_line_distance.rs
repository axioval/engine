//! Centre-line distances from fixtures to the walls beside them, measured
//! on real meshes.
#![cfg(feature = "axiolid")]
#![allow(missing_docs)]

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval::axiolid::{AxiolidGeometry, AxiolidPlanSpanService};
use axioval::engine::{
    CapabilityEvaluation, CompiledRule, PlanSpanServiceHandle, RuleCapability, RuleContext,
    ServiceRegistry,
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
        CentreLineDistance.evaluate(
            &RuleContext {
                project: &project,
                services: &services,
            },
            &rule,
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
