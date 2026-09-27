//! Parking bays and wall spacing, judged on real meshes: each footprint's
//! own axes come from its least-area rectangle, measured by Axiolid.
#![cfg(feature = "axiolid")]
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval::axiolid::{
    AxiolidGeometry, AxiolidPlanAreaService, AxiolidPlanSpanService, AxiolidProximityService,
    AxiolidVerticalExtentService,
};
use axioval::engine::{
    CapabilityEvaluation, CompiledRule, CompleteRelationshipSelection, PlanAreaServiceHandle,
    PlanSpanServiceHandle, ProximityServiceHandle, RelationshipQuery, RelationshipSelectionError,
    RelationshipSelectionRequest, RelationshipSelectionService, RelationshipSelectionServiceHandle,
    RuleCapability, RuleContext, ServiceRegistry, TraversalDirection, VerticalExtentServiceHandle,
};
use axioval::ir::contract::{ParameterValue, Selector, Severity};
use axioval::ir::{Evidence, NotEvaluatedReason, Object, ObjectId, Project, RuleId, SourceId};
use axioval::rules::{ParkingBay, WallSpacing};

const CONTAINS: &str = "contains";

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

/// A closed, outward-oriented prism from `bottom` to `top` over the
/// counter-clockwise plan polygon `plan`.
fn prism(plan: &[[f64; 2]], bottom: f64, top: f64) -> TriMesh {
    let n = u32::try_from(plan.len()).unwrap();
    let mut points: Vec<Point3> = plan
        .iter()
        .map(|[x, y]| Point3::new(*x, *y, bottom))
        .collect();
    points.extend(plan.iter().map(|[x, y]| Point3::new(*x, *y, top)));
    let mut indices = Vec::new();
    for i in 1..n - 1 {
        indices.extend([0, i + 1, i]);
        indices.extend([n, n + i, n + i + 1]);
    }
    for i in 0..n {
        let j = (i + 1) % n;
        indices.extend([i, j, n + j, i, n + j, n + i]);
    }
    TriMesh::new(points, indices)
}

/// An axis-aligned plan rectangle.
fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<[f64; 2]> {
    vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}

/// A `length` x `width` rectangle centred at `centre`, its length turned
/// `degrees` from the x-axis.
fn turned(centre: [f64; 2], length: f64, width: f64, degrees: f64) -> Vec<[f64; 2]> {
    let (sin, cos) = degrees.to_radians().sin_cos();
    [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .map(|(a, b)| {
            let (u, v) = (a * length / 2.0, b * width / 2.0);
            [centre[0] + cos * u - sin * v, centre[1] + sin * u + cos * v]
        })
        .to_vec()
}

#[derive(Default)]
struct Scene {
    objects: Vec<Object>,
    geometry: AxiolidGeometry,
    edges: Vec<(ObjectId, ObjectId)>,
}

impl Scene {
    fn body(mut self, local: &str, kind: &str, plan: &[[f64; 2]], bottom: f64, top: f64) -> Self {
        self.objects.push(Object::new(id(local), kind));
        self.geometry = self.geometry.with_mesh(id(local), prism(plan, bottom, top));
        self
    }

    fn bodiless(mut self, local: &str, kind: &str) -> Self {
        self.objects.push(Object::new(id(local), kind));
        self.geometry = self.geometry.with_no_body(id(local));
        self
    }

    fn contains(mut self, container: &str, members: &[&str]) -> Self {
        for member in members {
            self.edges.push((id(container), id(member)));
        }
        self
    }

    fn check(
        self,
        capability: &dyn RuleCapability,
        subjects: &str,
        parameters: Vec<(&str, ParameterValue)>,
    ) -> CapabilityEvaluation {
        let rule = CompiledRule {
            id: RuleId::new("under-test").unwrap(),
            capability: capability.id().into(),
            severity: Severity::Error,
            selector: kind(subjects),
            parameters: parameters
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect::<BTreeMap<_, _>>(),
        };
        let project = Project::new(self.objects.clone()).unwrap();
        let geometry = self.geometry.clone();
        let mut services = ServiceRegistry::new();
        services
            .register(PlanSpanServiceHandle::new(Arc::new(
                AxiolidPlanSpanService::new(geometry.clone(), source()),
            )))
            .unwrap();
        services
            .register(PlanAreaServiceHandle::new(Arc::new(
                AxiolidPlanAreaService::new(geometry.clone(), source()),
            )))
            .unwrap();
        services
            .register(ProximityServiceHandle::new(Arc::new(
                AxiolidProximityService::new(geometry.clone()),
            )))
            .unwrap();
        services
            .register(VerticalExtentServiceHandle::new(Arc::new(
                AxiolidVerticalExtentService::new(geometry),
            )))
            .unwrap();
        services
            .register(RelationshipSelectionServiceHandle::new(Arc::new(self)))
            .unwrap();
        capability.evaluate(
            &RuleContext {
                project: &project,
                services: &services,
            },
            &rule,
        )
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
        if relationship.as_str() != CONTAINS {
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
            vec![Evidence::exact(source(), "scan:contains")],
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

fn quantity(value: f64, unit: &str) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: unit.into(),
    }
}

fn metres(value: f64) -> ParameterValue {
    quantity(value, "m")
}

fn path(steps: &[&str]) -> ParameterValue {
    ParameterValue::StringList {
        value: steps.iter().map(|step| (*step).to_owned()).collect(),
    }
}

/// `(object, message, related)` of every finding, sorted.
fn findings(outcome: &CapabilityEvaluation) -> Vec<(String, String, Vec<String>)> {
    let mut found: Vec<_> = outcome
        .findings()
        .iter()
        .map(|finding| {
            (
                finding
                    .object_id()
                    .map_or_else(String::new, |object| object.local_id.clone()),
                finding.message.clone(),
                finding
                    .related
                    .iter()
                    .map(|related| related.local_id.clone())
                    .collect(),
            )
        })
        .collect();
    found.sort();
    found
}

/// A 30 m aisle along x (y 0..6) with bays north of it, 2.2 m high:
///
/// - `b1`: 2.5 x 5 m, square to the aisle, x 0..2.5;
/// - `b2`: 2.5 x 4.8 m turned 45 degrees at x 10: its bounding box is
///   5.16 m square, longer than 5 m both ways, its own length is not;
/// - `c1`: a column 0.1 m west of `b1`'s side, `c2` one 0.05 m past its far
///   end, `c3` one far away.
fn car_park() -> Scene {
    let b2 = turned([10.0, 6.0 + 7.3 / 2.0_f64.sqrt() / 2.0], 4.8, 2.5, 45.0);
    Scene::default()
        .body("aisle", "aisle", &rect(-5.0, 0.0, 25.0, 6.0), 0.0, 2.5)
        .body("b1", "bay", &rect(0.0, 6.0, 2.5, 11.0), 0.0, 2.2)
        .body("b2", "bay", &b2, 0.0, 2.2)
        .body("c1", "column", &rect(-0.5, 8.0, -0.1, 8.4), 0.0, 3.0)
        .body("c2", "column", &rect(1.0, 11.05, 1.4, 11.45), 0.0, 3.0)
        .body("c3", "column", &rect(20.0, 20.0, 20.4, 20.4), 0.0, 3.0)
}

#[test]
fn a_bay_too_short_along_its_own_axis_is_found_though_its_box_is_long_enough() {
    let outcome = car_park().check(
        &ParkingBay,
        "bay",
        vec![
            ("min_width", metres(2.4)),
            ("min_length", metres(5.0)),
            ("min_height", metres(2.1)),
        ],
    );
    let found = findings(&outcome);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].0, "b2");
    assert!(
        found[0]
            .1
            .starts_with("length along the bay's own axes is 4.8 m; at least 5 m"),
        "{found:#?}"
    );
    assert!(outcome.not_evaluated_outcomes().is_empty(), "{outcome:?}");
}

#[test]
fn a_bay_at_the_wrong_angle_to_its_aisle_is_found() {
    let orientation = |alignment: &str| {
        vec![
            ("aisles", selector(kind("aisle"))),
            ("orientation", text(alignment)),
            ("angle_tolerance", quantity(5.0, "deg")),
        ]
    };
    let found = findings(&car_park().check(&ParkingBay, "bay", orientation("perpendicular")));
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].0, "b2");
    assert!(
        found[0]
            .1
            .starts_with("the bay is not perpendicular to any aisle"),
        "{found:#?}"
    );
    assert_eq!(found[0].2, ["aisle"]);
    let found = findings(&car_park().check(&ParkingBay, "bay", orientation("angled")));
    assert_eq!(
        found.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(),
        ["b1"]
    );
}

#[test]
fn obstructed_ends_and_sides_are_counted_against_what_is_allowed() {
    let obstructions = |ends: &str, sides: &str| {
        vec![
            ("obstacles", selector(kind("column"))),
            ("obstruction_reach", metres(0.2)),
            ("end_obstructions", text(ends)),
            ("side_obstructions", text(sides)),
        ]
    };
    let outcome = car_park().check(&ParkingBay, "bay", obstructions("none", "one"));
    let found = findings(&outcome);
    assert_eq!(
        found,
        [(
            "b1".to_owned(),
            "1 of its ends obstructed by cad:model/c2, none allowed".to_owned(),
            vec!["c2".to_owned()],
        )],
        "{found:#?}"
    );
    assert!(outcome.not_evaluated_outcomes().is_empty(), "{outcome:?}");
    let found = findings(&car_park().check(&ParkingBay, "bay", obstructions("one", "none")));
    assert_eq!(
        found,
        [(
            "b1".to_owned(),
            "1 of its sides obstructed by cad:model/c1, none allowed".to_owned(),
            vec!["c1".to_owned()],
        )],
        "{found:#?}"
    );
    // Within 8 cm, c1 (10 cm off) is no obstruction; c2 still is.
    let mut near = obstructions("none", "none");
    near[1].1 = metres(0.08);
    let found = findings(&car_park().check(&ParkingBay, "bay", near));
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].1.contains("ends"), "{found:#?}");
}

#[test]
fn a_square_bay_has_sides_but_no_ends() {
    let scene = Scene::default()
        .body("aisle", "aisle", &rect(-5.0, 0.0, 25.0, 6.0), 0.0, 2.5)
        .body("sq", "bay", &rect(0.0, 6.0, 3.0, 9.0), 0.0, 2.2)
        .body("c1", "column", &rect(1.0, 9.05, 1.4, 9.45), 0.0, 3.0);
    let outcome = scene.check(
        &ParkingBay,
        "bay",
        vec![
            ("min_width", metres(2.5)),
            ("obstacles", selector(kind("column"))),
            ("obstruction_reach", metres(0.2)),
            ("end_obstructions", text("none")),
            ("side_obstructions", text("both")),
        ],
    );
    // Its width is judged; which of its edges are ends is not.
    assert!(outcome.findings().is_empty(), "{outcome:?}");
    let outcomes = outcome.not_evaluated_outcomes();
    assert_eq!(outcomes.len(), 1, "{outcomes:?}");
    assert_eq!(
        outcomes[0].reason(),
        &NotEvaluatedReason::IncompleteEvidence
    );
    assert!(
        outcomes[0].message().contains("too close to equal"),
        "{outcomes:?}"
    );
}

#[test]
fn an_undeclared_check_or_a_half_declared_one_is_refused() {
    for parameters in [
        vec![],
        vec![("orientation", text("parallel"))],
        vec![
            ("obstacles", selector(kind("column"))),
            ("end_obstructions", text("none")),
        ],
        vec![("min_length", metres(5.0)), ("max_length", metres(4.0))],
        vec![
            ("aisles", selector(kind("aisle"))),
            ("orientation", text("parallel")),
            ("angle_tolerance", quantity(50.0, "deg")),
        ],
    ] {
        let outcome = car_park().check(&ParkingBay, "bay", parameters);
        assert!(outcome.findings().is_empty());
        assert_eq!(
            outcome.not_evaluated_outcomes()[0].reason(),
            &NotEvaluatedReason::InvalidDeclaration
        );
    }
}

/// A storey over a 10 x 8 m slab, with walls 0.2 m thick along x: `s` at
/// y 0..0.2, `m` at y 5..5.2 from x 2, `n` at y 5.6..5.8, and `x` across
/// them at x 0..0.2.
fn storey() -> Scene {
    Scene::default()
        .bodiless("st", "storey")
        .body("slab", "slab", &rect(0.0, 0.0, 10.0, 8.0), -0.3, 0.0)
        .body("s", "wall", &rect(0.0, 0.0, 10.0, 0.2), 0.0, 3.0)
        .body("m", "wall", &rect(2.0, 5.0, 10.0, 5.2), 0.0, 3.0)
        .body("n", "wall", &rect(0.0, 5.6, 10.0, 5.8), 0.0, 3.0)
        .body("x", "wall", &rect(0.0, 0.2, 0.2, 5.6), 0.0, 3.0)
        .contains("st", &["slab", "s", "m", "n", "x"])
}

fn spacing(extra: Vec<(&'static str, ParameterValue)>) -> Vec<(&'static str, ParameterValue)> {
    let mut parameters = vec![
        ("members", selector(kind("wall"))),
        ("member_path", path(&[CONTAINS])),
        ("angle_tolerance", quantity(5.0, "deg")),
    ];
    parameters.extend(extra);
    parameters
}

#[test]
fn two_parallel_walls_too_close_are_found() {
    let outcome = storey().check(
        &WallSpacing,
        "storey",
        spacing(vec![("minimum", metres(1.0))]),
    );
    let found = findings(&outcome);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].0, "st");
    assert!(
        found[0]
            .1
            .starts_with("cad:model/m and cad:model/n are parallel and 0.4 m apart in plan"),
        "{found:#?}"
    );
    assert_eq!(found[0].2, ["m", "n"]);
    assert!(outcome.not_evaluated_outcomes().is_empty(), "{outcome:?}");
}

#[test]
fn an_uncovered_strip_of_the_storey_is_found() {
    let coverage = |maximum: f64| {
        spacing(vec![
            ("maximum", metres(maximum)),
            ("footprints", selector(kind("slab"))),
            ("footprint_path", path(&[CONTAINS])),
            ("uncovered_above", quantity(1.0, "m2")),
        ])
    };
    // s and n, 5.4 m apart, cover y 0..5.8; the strip north of n is left.
    let outcome = storey().check(&WallSpacing, "storey", coverage(6.0));
    let found = findings(&outcome);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0]
            .1
            .starts_with("22 m² of cad:model/slab lies outside every band"),
        "{found:#?}"
    );
    assert!(found[0].2.contains(&"slab".to_owned()), "{found:#?}");
    assert!(outcome.not_evaluated_outcomes().is_empty(), "{outcome:?}");
    // At most 5 m apart, s and n are no band: only m and n (x 2..10) and
    // s and m (x 2..10, y 0..5.2) are.
    let found = findings(&storey().check(&WallSpacing, "storey", coverage(5.0)));
    assert!(
        found[0].1.starts_with("33.6 m² of cad:model/slab"),
        "{found:#?}"
    );
}
