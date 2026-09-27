//! Passing spaces along accessible routes, searched on real meshes.
//!
//! A lobby `a` (x 0..2) opens onto a corridor `b` (x 2.2..33.8), 1.8 m
//! wide, which ends in a room `e` (x 34..36). Walls 0.2 m thick separate
//! them, each with a bodiless opening 1.6 m wide.
//! Cabinets 0.8 m deep along the corridor's north wall leave 1 m free: a
//! 0.9 m body passes everywhere, a 1.5 m square passing space only where
//! the cabinets leave a gap.
#![cfg(feature = "axiolid")]
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval::axiolid::{
    AxiolidFreeSpaceService, AxiolidGeometry, AxiolidMetricRoutingService, AxiolidPlanSpanService,
    AxiolidVerticalExtentService, AxiolidWalkabilityService,
};
use axioval::engine::{
    CapabilityEvaluation, CompiledRule, FreeSpaceServiceHandle, MetricRoutingServiceHandle,
    PlanSpanServiceHandle, RuleCapability, RuleContext, ServiceRegistry,
    VerticalExtentServiceHandle, WalkabilityServiceHandle,
};
use axioval::ir::contract::{ParameterValue, Selector, Severity};
use axioval::ir::{NotEvaluatedReason, Object, ObjectId, Project, RuleId, SourceId};
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
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7,
            6, 3, 0, 4, 3, 4, 7,
        ],
    )
}

const OPENINGS: [(&str, f64); 2] = [("west", 2.0), ("east", 33.8)];

/// The void of an opening whose wall starts at `x0`.
fn void(x0: f64) -> TriMesh {
    cuboid([x0, 0.1, 0.0], [x0 + 0.2, 1.7, 2.1])
}

struct Scene {
    objects: Vec<Object>,
    geometry: AxiolidGeometry,
    metric: bool,
}

impl Scene {
    /// The spaces, with cabinets along the corridor from x to x.
    fn new(cabinets: &[(f64, f64)]) -> Self {
        let mut scene = Self {
            objects: Vec::new(),
            geometry: AxiolidGeometry::new(),
            metric: true,
        };
        for (local, kind, x0, x1) in [
            ("a", "lobby", 0.0, 2.0),
            ("b", "corridor", 2.2, 33.8),
            ("e", "room", 34.0, 36.0),
        ] {
            scene = scene.body(local, kind, cuboid([x0, 0.0, 0.0], [x1, 1.8, 3.0]));
        }
        for (opening, x0) in OPENINGS {
            let x1 = x0 + 0.2;
            scene = scene
                .body(
                    &format!("{opening}-s"),
                    "wall",
                    cuboid([x0, 0.0, 0.0], [x1, 0.1, 3.0]),
                )
                .body(
                    &format!("{opening}-n"),
                    "wall",
                    cuboid([x0, 1.7, 0.0], [x1, 1.8, 3.0]),
                )
                .body(
                    &format!("{opening}-lintel"),
                    "wall",
                    cuboid([x0, 0.1, 2.1], [x1, 1.7, 3.0]),
                );
            scene.objects.push(Object::new(id(opening), "opening"));
            scene.geometry = scene.geometry.with_no_body(id(opening));
        }
        for (index, (x0, x1)) in cabinets.iter().enumerate() {
            scene = scene.body(
                &format!("cabinet-{index}"),
                "cabinet",
                cuboid([*x0, 1.0, 0.0], [*x1, 1.8, 2.0]),
            );
        }
        scene
    }

    fn body(mut self, local: &str, kind: &str, mesh: TriMesh) -> Self {
        self.objects.push(Object::new(id(local), kind));
        self.geometry = self.geometry.with_mesh(id(local), mesh);
        self
    }

    fn check(self, parameters: &[(&str, ParameterValue)]) -> CapabilityEvaluation {
        let mut bound: BTreeMap<String, ParameterValue> = BTreeMap::from([
            (
                "route_selector".to_owned(),
                selector(Selector::AnyOf {
                    operands: vec![kind("lobby"), kind("corridor"), kind("room")],
                }),
            ),
            ("start_selector".to_owned(), selector(kind("lobby"))),
            ("portal_selector".to_owned(), selector(kind("opening"))),
            (
                "obstacle_selector".to_owned(),
                selector(Selector::AnyOf {
                    operands: vec![kind("cabinet"), kind("wall")],
                }),
            ),
            ("width_metres".to_owned(), number(0.9)),
            ("clear_height_metres".to_owned(), number(2.1)),
            ("passing_width_metres".to_owned(), number(1.5)),
            ("passing_length_metres".to_owned(), number(1.5)),
            ("passing_spacing_metres".to_owned(), number(15.0)),
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
        let geometry = self.geometry;
        let mut services = ServiceRegistry::new();
        let mut walkability = AxiolidWalkabilityService::new(geometry.clone(), source());
        for (opening, x0) in OPENINGS {
            walkability = walkability.with_opening_void(id(opening), void(x0));
        }
        services
            .register(WalkabilityServiceHandle::new(Arc::new(walkability)))
            .unwrap();
        if self.metric {
            let mut routes = AxiolidMetricRoutingService::new(geometry.clone(), source());
            for surface in ["a", "b", "e"] {
                routes = routes.with_surface(id(surface));
            }
            for (opening, x0) in OPENINGS {
                routes = routes
                    .with_portal(id(opening))
                    .with_opening_void(id(opening), void(x0));
            }
            services
                .register(MetricRoutingServiceHandle::new(Arc::new(routes)))
                .unwrap();
        }
        services
            .register(PlanSpanServiceHandle::new(Arc::new(
                AxiolidPlanSpanService::new(geometry.clone(), source()),
            )))
            .unwrap();
        services
            .register(VerticalExtentServiceHandle::new(Arc::new(
                AxiolidVerticalExtentService::new(geometry.clone()),
            )))
            .unwrap();
        services
            .register(FreeSpaceServiceHandle::new(Arc::new(
                AxiolidFreeSpaceService::new(geometry, source()),
            )))
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

fn unevaluated(outcome: &CapabilityEvaluation) -> Vec<(NotEvaluatedReason, String)> {
    outcome
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| (outcome.reason().clone(), outcome.message().to_owned()))
        .collect()
}

#[test]
fn gaps_between_cabinets_every_six_metres_are_passing_spaces() {
    let outcome = Scene::new(&[
        (4.0, 8.0),
        (10.0, 14.0),
        (16.0, 20.0),
        (22.0, 26.0),
        (28.0, 32.0),
    ])
    .check(&[]);
    assert!(outcome.findings().is_empty(), "{outcome:#?}");
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
}

#[test]
fn a_cabinet_run_longer_than_the_spacing_leaves_no_passing_space() {
    let outcome = Scene::new(&[(4.0, 32.0)]).check(&[]);
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
    let findings = outcome.findings();
    assert_eq!(findings.len(), 1, "{outcome:#?}");
    let message = &findings[0].message;
    assert!(
        message.starts_with("no route to cad:model/e has its passing spaces: the route from cad:model/a has no passing space (1.5 m by 1.5 m) between"),
        "{message}"
    );
    assert!(
        message.ends_with("a stretch longer than the 15 m allowed"),
        "{message}"
    );
    assert_eq!(findings[0].related, [id("a")]);
    // A longer spacing is met: the lobby and the room are passing spaces.
    let outcome = Scene::new(&[(4.0, 32.0)]).check(&[("passing_spacing_metres", number(40.0))]);
    assert!(outcome.findings().is_empty(), "{outcome:#?}");
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
}

#[test]
fn a_gap_neither_proven_nor_ruled_out_is_not_evaluated() {
    // Gaps every 9 m: no stretch longer than 15 m lacks one, but the route
    // is searched in tiles of 6.8 m, and the last gap is too far from the
    // room to show either way.
    let outcome = Scene::new(&[(4.0, 11.0), (13.0, 20.0), (22.0, 29.0)]).check(&[]);
    assert!(outcome.findings().is_empty(), "{outcome:#?}");
    let open = unevaluated(&outcome);
    assert_eq!(open.len(), 1, "{outcome:#?}");
    assert_eq!(open[0].0, NotEvaluatedReason::IncompleteEvidence);
    assert!(
        open[0]
            .1
            .contains("no passing space (1.5 m by 1.5 m) is proven"),
        "{open:#?}"
    );
}

#[test]
fn passing_spaces_fail_closed() {
    let mut scene = Scene::new(&[(4.0, 32.0)]);
    scene.metric = false;
    let outcome = scene.check(&[]);
    assert!(outcome.findings().is_empty(), "{outcome:#?}");
    let open = unevaluated(&outcome);
    assert_eq!(open.len(), 1, "{outcome:#?}");
    assert_eq!(open[0].0, NotEvaluatedReason::MissingService);
    // A passing space needs its height and all three dimensions.
    let outcome = Scene::new(&[]).check(&[("passing_length_metres", number(-1.0))]);
    assert_eq!(
        unevaluated(&outcome)[0].0,
        NotEvaluatedReason::InvalidDeclaration
    );
    let outcome = Scene::new(&[]).check(&[(
        "passing_width_metres",
        ParameterValue::String {
            value: "wide".into(),
        },
    )]);
    assert_eq!(
        unevaluated(&outcome)[0].0,
        NotEvaluatedReason::InvalidDeclaration
    );
}
