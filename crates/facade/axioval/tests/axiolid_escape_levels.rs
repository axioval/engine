//! Escape routes down a stair, walked over real Axiolid geometry.
//!
//! A ground-floor room `ground` (x 0..10, y 0..4, 2.7 m high) has its only
//! exit, `exit`, in its east wall (y 3..3.9). A straight stair of twelve
//! 0.25 m risers, 0.3 m goings and 1.2 m width climbs along +x from x = 2
//! (y 1..2.2) to x = 5.6 and z = 3, where the second-floor room `upper`
//! (same plan) begins. `upper` reaches the exit only down the stair: about
//! 6.1 m to the stair's head, 4.7 m along its slope and 8.8 m on to the
//! exit.
#![cfg(feature = "axiolid")]
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval::axiolid::{
    AxiolidGeometry, AxiolidMetricRoutingService, AxiolidPlanSpanService,
    AxiolidVerticalExtentService,
};
use axioval::engine::{
    CapabilityEvaluation, CompiledRule, CompleteRelationshipSelection, MetricRoutingServiceHandle,
    PlanSpanServiceHandle, RelationshipQuery, RelationshipSelectionError,
    RelationshipSelectionRequest, RelationshipSelectionService, RelationshipSelectionServiceHandle,
    RuleCapability, RuleContext, ServiceRegistry, VerticalExtentServiceHandle,
};
use axioval::ir::contract::{ParameterValue, Selector, Severity, TableRow};
use axioval::ir::{Evidence, Object, ObjectId, Project, RuleId, Scope, SourceId};
use axioval::rules::EscapeRoute;

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

/// The stair: its side profile (x, z) swept from y 1 to 2.2.
fn stair() -> TriMesh {
    let (steps, rise, going, start) = (12_u32, 0.25, 0.3, 2.0);
    let length = going * f64::from(steps);
    let mut profile = vec![[start, 0.0], [start + length, 0.0], [start + length, 3.0]];
    let mut elevation = 3.0;
    for step in (0..steps).rev() {
        let front = start + going * f64::from(step);
        profile.push([front, elevation]);
        elevation -= rise;
        if step > 0 {
            profile.push([front, elevation]);
        }
    }
    let n = profile.len();
    let mut points: Vec<Point3> = profile
        .iter()
        .map(|p| Point3::new(p[0], 1.0, p[1]))
        .collect();
    points.extend(profile.iter().map(|p| Point3::new(p[0], 2.2, p[1])));
    let mut indices: Vec<u32> = Vec::new();
    for [a, b, c] in ears(&profile) {
        indices.extend([a, b, c].map(|i| u32::try_from(i).unwrap()));
        indices.extend([a + n, c + n, b + n].map(|i| u32::try_from(i).unwrap()));
    }
    for i in 0..n {
        let j = (i + 1) % n;
        indices.extend([i, j + n, j, i, i + n, j + n].map(|k| u32::try_from(k).unwrap()));
    }
    TriMesh::new(points, indices)
}

/// Ear-clipping triangulation of a simple counter-clockwise polygon.
fn ears(polygon: &[[f64; 2]]) -> Vec<[usize; 3]> {
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut remaining: Vec<usize> = (0..polygon.len()).collect();
    let mut found = Vec::new();
    while remaining.len() > 3 {
        let count = remaining.len();
        let ear = (0..count)
            .find(|&i| {
                let (p, c, n) = (
                    remaining[(i + count - 1) % count],
                    remaining[i],
                    remaining[(i + 1) % count],
                );
                if cross(polygon[p], polygon[c], polygon[n]) <= 0.0 {
                    return false;
                }
                remaining.iter().all(|&other| {
                    other == p
                        || other == c
                        || other == n
                        || cross(polygon[p], polygon[c], polygon[other]) < 0.0
                        || cross(polygon[c], polygon[n], polygon[other]) < 0.0
                        || cross(polygon[n], polygon[p], polygon[other]) < 0.0
                })
            })
            .expect("a simple polygon has an ear");
        found.push([
            remaining[(ear + count - 1) % count],
            remaining[ear],
            remaining[(ear + 1) % count],
        ]);
        remaining.remove(ear);
    }
    found.push([remaining[0], remaining[1], remaining[2]]);
    found
}

/// Each space names the exit it escapes through.
struct Exits;

impl RelationshipSelectionService for Exits {
    fn select(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        let RelationshipQuery::Related { relationship, .. } = request.query() else {
            return Err(RelationshipSelectionError::InvalidRequest);
        };
        assert_eq!(relationship.as_str(), "escapes");
        let candidates = request
            .candidate_universe()
            .iter()
            .filter(|candidate| **candidate == id("exit"))
            .cloned()
            .collect();
        CompleteRelationshipSelection::try_new(
            request.clone(),
            candidates,
            vec![Evidence::exact(source(), "escapes")],
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

fn check(maximum: f64, climbing: bool) -> CapabilityEvaluation {
    let mut objects = Vec::new();
    let mut geometry = AxiolidGeometry::new();
    for (local, kind, mesh) in [
        ("ground", "space", cuboid([0.0, 0.0, 0.0], [10.0, 4.0, 2.7])),
        ("upper", "space", cuboid([0.0, 0.0, 3.0], [10.0, 4.0, 5.7])),
        ("stair", "stair", stair()),
        ("wall-s", "wall", cuboid([10.0, 0.0, 0.0], [10.2, 3.0, 2.7])),
        ("wall-n", "wall", cuboid([10.0, 3.9, 0.0], [10.2, 4.0, 2.7])),
        ("lintel", "wall", cuboid([10.0, 3.0, 2.1], [10.2, 3.9, 2.7])),
        ("exit", "door", cuboid([10.05, 3.0, 0.0], [10.15, 3.9, 2.1])),
    ] {
        objects.push(Object::new(id(local), kind));
        geometry = geometry.with_mesh(id(local), mesh);
    }
    let row: TableRow = [
        ("spaces".to_owned(), selector(kind("space"))),
        ("maximum_travel".to_owned(), number(maximum)),
    ]
    .into_iter()
    .collect();
    let mut parameters: BTreeMap<String, ParameterValue> = BTreeMap::from([
        (
            "uses".to_owned(),
            ParameterValue::Table { value: vec![row] },
        ),
        (
            "exit_path".to_owned(),
            ParameterValue::StringList {
                value: vec!["escapes:forward".into()],
            },
        ),
        ("exit_selector".to_owned(), selector(kind("door"))),
        ("walking_height".to_owned(), number(2.0)),
        ("walking_step".to_owned(), number(0.02)),
    ]);
    if climbing {
        parameters.insert("stair_selector".to_owned(), selector(kind("stair")));
    }
    let rule = CompiledRule {
        id: RuleId::new("escape-route").unwrap(),
        capability: "axioval:capability.escape-route".into(),
        severity: Severity::Error,
        selector: kind("space"),
        parameters,
    };
    let project = Project::new(objects).unwrap();
    let routes = AxiolidMetricRoutingService::new(geometry.clone(), source())
        .with_surface(id("ground"))
        .with_surface(id("upper"))
        .with_portal(id("exit"));
    let mut services = ServiceRegistry::new();
    services
        .register(MetricRoutingServiceHandle::new(Arc::new(routes)))
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
        .register(RelationshipSelectionServiceHandle::new(Arc::new(Exits)))
        .unwrap();
    EscapeRoute.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        &rule,
    )
}

fn on(outcome: &CapabilityEvaluation, local: &str) -> (Vec<String>, Vec<String>) {
    let mine = |scope: &Scope| matches!(scope, Scope::Object(object) if object.local_id == local);
    (
        outcome
            .findings()
            .iter()
            .filter(|finding| mine(&finding.scope))
            .map(|finding| finding.message.clone())
            .collect(),
        outcome
            .not_evaluated_outcomes()
            .iter()
            .filter(|outcome| mine(outcome.scope()))
            .map(|outcome| outcome.message().to_owned())
            .collect(),
    )
}

#[test]
fn an_upper_room_whose_stair_route_to_the_only_exit_is_too_long_is_found() {
    // About 19.6 m from the far corner upstairs, down the stair, to the
    // exit; the ground room's farthest point lies about 10.7 m from it.
    let outcome = check(15.0, true);
    let (found, open) = on(&outcome, "upper");
    assert!(open.is_empty(), "{outcome:?}");
    assert_eq!(found.len(), 1, "{outcome:?}");
    assert!(
        found[0].starts_with("its farthest point, around (0.00, 4.00), lies "),
        "{found:?}"
    );
    let (found, open) = on(&outcome, "ground");
    assert!(found.is_empty() && open.is_empty(), "{outcome:?}");

    // Allowed 25 m, both rooms pass.
    let outcome = check(25.0, true);
    assert!(outcome.findings().is_empty(), "{outcome:?}");
    assert!(outcome.not_evaluated_outcomes().is_empty(), "{outcome:?}");
}

#[test]
fn without_the_stair_the_upper_room_reaches_no_exit_and_is_not_guessed() {
    // The host declares no connector, so the upper room's level is closed
    // and the exit below lies off it: a proven cut-off, found.
    let outcome = check(25.0, false);
    let (found, _) = on(&outcome, "upper");
    assert_eq!(found.len(), 1, "{outcome:?}");
    assert!(found[0].contains("reaches no exit"), "{found:?}");
}
