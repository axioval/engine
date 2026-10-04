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
use axioval::engine::{PropertyResolution, measured_value};
use axioval::ir::contract::{ParameterValue, Selector, Severity};
use axioval::ir::{
    Evidence, NotEvaluatedReason, Object, ObjectId, Project, PropertyValue, RuleId, SourceId,
};
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
    edges: Vec<(&'static str, ObjectId, ObjectId)>,
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

    fn contains(self, container: &str, members: &[&str]) -> Self {
        self.relate(CONTAINS, container, members)
    }

    fn relate(mut self, relationship: &'static str, from: &str, to: &[&str]) -> Self {
        for member in to {
            self.edges.push((relationship, id(from), id(member)));
        }
        self
    }

    /// The geometry services and this scene's relationships.
    fn services(self) -> (Project, ServiceRegistry) {
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
        (project, services)
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
        let (project, services) = self.services();
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
        if !self
            .edges
            .iter()
            .any(|(name, _, _)| *name == relationship.as_str())
        {
            return Err(RelationshipSelectionError::Unavailable(format!(
                "no {} in this source",
                relationship.as_str()
            )));
        }
        let reached = self
            .edges
            .iter()
            .filter(|(name, from, to)| {
                *name == relationship.as_str()
                    && from == request.anchor()
                    && request.candidate_universe().contains(to)
            })
            .map(|(_, _, to)| to.clone())
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

/// A row of three perpendicular bays north of the aisle, 2.5 m wide and
/// 4.8 m long (`p1` to `p3`, x 0 to 7.5), and a parallel bay `q` along the
/// aisle's south side, 6 x 2.4 m, 5.5 m long; column `k` stands off the
/// north-west corner of `p1`, beside its west side but only within 0.3 m of
/// its far end.
fn mixed_bays() -> Scene {
    Scene::default()
        .body("aisle", "aisle", &rect(-5.0, 0.0, 25.0, 6.0), 0.0, 2.5)
        .body("p1", "bay", &rect(0.0, 6.0, 2.5, 10.8), 0.0, 2.2)
        .body("p2", "bay", &rect(2.5, 6.0, 5.0, 10.8), 0.0, 2.2)
        .body("p3", "bay", &rect(5.0, 6.0, 7.5, 10.8), 0.0, 2.2)
        .body("q", "bay", &rect(10.0, -2.4, 15.5, 0.0), 0.0, 2.2)
        .body("k", "column", &rect(-0.4, 10.5, -0.05, 11.0), 0.0, 3.0)
}

#[test]
fn a_size_bound_applies_only_to_bays_in_the_selected_orientation() {
    let filtered = |orientations: &[&str]| {
        vec![
            ("min_length", metres(5.0)),
            ("applies_when", text("filter")),
            ("orientations", path(orientations)),
            ("aisles", selector(kind("aisle"))),
            ("angle_tolerance", quantity(5.0, "deg")),
        ]
    };
    // The perpendicular bays are 4.8 m long, short of 5 m; the parallel bay
    // is 5.5 m long, and ignored anyway.
    let outcome = mixed_bays().check(&ParkingBay, "bay", filtered(&["perpendicular"]));
    let found = findings(&outcome);
    assert_eq!(
        found.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(),
        ["p1", "p2", "p3"],
        "{found:#?}"
    );
    assert!(
        found[0].1.starts_with(
            "length along the bay's own axes is 4.8 m; at least 5 m (a bay with orientation \
             perpendicular)"
        ),
        "{found:#?}"
    );
    assert!(outcome.not_evaluated_outcomes().is_empty(), "{outcome:?}");
    // Parallel bays alone: the one there is long enough.
    let outcome = mixed_bays().check(&ParkingBay, "bay", filtered(&["parallel"]));
    assert!(outcome.findings().is_empty(), "{outcome:?}");
    assert!(outcome.not_evaluated_outcomes().is_empty(), "{outcome:?}");
}

#[test]
fn without_an_aisle_the_orientation_is_inferred_from_neighbouring_bays() {
    let filtered = |orientations: &[&str]| {
        vec![
            ("min_length", metres(5.0)),
            ("applies_when", text("filter")),
            ("orientations", path(orientations)),
            ("neighbour_reach", metres(0.05)),
            ("angle_tolerance", quantity(5.0, "deg")),
        ]
    };
    // The row's bays stand side by side: perpendicular. `q` has no
    // neighbour: unclear.
    let found = findings(&mixed_bays().check(&ParkingBay, "bay", filtered(&["perpendicular"])));
    assert_eq!(
        found.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(),
        ["p1", "p2", "p3"],
        "{found:#?}"
    );
    let outcome = mixed_bays().check(&ParkingBay, "bay", filtered(&["unclear"]));
    assert!(outcome.findings().is_empty(), "{outcome:?}");
    assert!(outcome.not_evaluated_outcomes().is_empty(), "{outcome:?}");
    // Two bays end to end are parallel.
    let row = Scene::default()
        .body("e1", "bay", &rect(0.0, 0.0, 4.8, 2.4), 0.0, 2.2)
        .body("e2", "bay", &rect(4.8, 0.0, 9.6, 2.4), 0.0, 2.2);
    let found = findings(&row.check(&ParkingBay, "bay", filtered(&["parallel"])));
    assert_eq!(found.len(), 2, "{found:#?}");
}

#[test]
fn a_corner_column_outside_the_side_zone_is_no_obstruction() {
    let states = |zone: Option<f64>| {
        let mut parameters = vec![
            ("min_width", metres(2.6)),
            ("applies_when", text("filter")),
            ("side_states", path(&["none"])),
            ("obstacles", selector(kind("column"))),
            ("obstruction_reach", metres(0.1)),
        ];
        if let Some(zone) = zone {
            parameters.push(("side_zone_length", metres(zone)));
        }
        parameters
    };
    // Along the whole side, `k` obstructs `p1`'s west side, so the 2.6 m
    // width required of unobstructed bays applies to the others only.
    let found = findings(&mixed_bays().check(&ParkingBay, "bay", states(None)));
    assert_eq!(
        found.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(),
        ["p2", "p3", "q"],
        "{found:#?}"
    );
    // Within the central 3 m of the side, it does not: `p1` is judged too.
    let outcome = mixed_bays().check(&ParkingBay, "bay", states(Some(3.0)));
    let found = findings(&outcome);
    assert_eq!(
        found.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(),
        ["p1", "p2", "p3", "q"],
        "{found:#?}"
    );
    assert!(outcome.not_evaluated_outcomes().is_empty(), "{outcome:?}");
    // In findings mode the zone counts the same way.
    let findings_mode = |zone: f64| {
        vec![
            ("obstacles", selector(kind("column"))),
            ("obstruction_reach", metres(0.1)),
            ("end_obstructions", text("both")),
            ("side_obstructions", text("none")),
            ("side_zone_length", metres(zone)),
        ]
    };
    assert!(findings(&mixed_bays().check(&ParkingBay, "bay", findings_mode(3.0))).is_empty());
    assert_eq!(
        findings(&mixed_bays().check(&ParkingBay, "bay", findings_mode(4.8))).len(),
        1
    );
}

#[test]
fn filter_declarations_are_checked() {
    for parameters in [
        vec![
            ("min_length", metres(5.0)),
            ("orientations", path(&["parallel"])),
        ],
        vec![
            ("min_length", metres(5.0)),
            ("applies_when", text("filter")),
        ],
        vec![
            ("applies_when", text("filter")),
            ("orientations", path(&["parallel"])),
            ("aisles", selector(kind("aisle"))),
            ("angle_tolerance", quantity(5.0, "deg")),
        ],
        vec![
            ("min_length", metres(5.0)),
            ("applies_when", text("filter")),
            ("orientations", path(&["sideways"])),
            ("aisles", selector(kind("aisle"))),
            ("angle_tolerance", quantity(5.0, "deg")),
        ],
        vec![
            ("min_length", metres(5.0)),
            ("applies_when", text("filter")),
            ("orientations", path(&["parallel"])),
        ],
        vec![
            ("min_length", metres(5.0)),
            ("applies_when", text("filter")),
            ("end_states", path(&["none"])),
        ],
        vec![
            ("min_length", metres(5.0)),
            ("applies_when", text("filter")),
            ("side_states", path(&["none"])),
            ("obstacles", selector(kind("column"))),
            ("obstruction_reach", metres(0.1)),
            ("side_obstructions", text("none")),
        ],
    ] {
        let outcome = mixed_bays().check(&ParkingBay, "bay", parameters.clone());
        assert!(outcome.findings().is_empty(), "{parameters:?}");
        assert_eq!(
            outcome.not_evaluated_outcomes()[0].reason(),
            &NotEvaluatedReason::InvalidDeclaration,
            "{parameters:?}"
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

/// The measured value `name` of `local` in `scene`, in degrees as
/// `(lower, upper)`.
fn degrees(scene: Scene, local: &str, name: &str) -> (f64, f64) {
    let (project, services) = scene.services();
    let PropertyResolution::Present(resolved) =
        measured_value(&services, &project, &id(local), name).unwrap()
    else {
        panic!("{local} has no {name}");
    };
    match resolved.property().value() {
        PropertyValue::Quantity { value, .. } => (value.to_degrees(), value.to_degrees()),
        PropertyValue::Measured { lower, upper, .. } => (lower.to_degrees(), upper.to_degrees()),
        other => panic!("{name} of {local} is {other:?}"),
    }
}

fn near((lower, upper): (f64, f64), value: f64) -> bool {
    lower - 1e-9 <= value && value <= upper + 1e-9 && upper - lower < 1e-6
}

#[test]
fn measured_angles_agree_with_the_bay_orientation_judgement() {
    let scene = || {
        car_park()
            .relate("serves", "b1", &["aisle"])
            .relate("serves", "b2", &["aisle"])
    };
    // `perpendicular` within 5° finds `b2` only, and `angled` only `b1`:
    // `b1` stands at 90° to its aisle and `b2` at 45°.
    let b1 = degrees(scene(), "b1", "angle_to;path=serves");
    let b2 = degrees(scene(), "b2", "angle_to;path=serves");
    assert!(near(b1, 90.0), "{b1:?}");
    assert!(near(b2, 45.0), "{b2:?}");
    assert!(near(degrees(scene(), "b1", "skew;path=serves"), 0.0));
    assert!(near(degrees(scene(), "b2", "skew;path=serves"), 45.0));
    // The long axis of `b1` runs north, the aisle's east; a long axis has
    // no direction.
    assert!(near(degrees(scene(), "b1", "bearing;axis=long"), 0.0));
    assert!(near(degrees(scene(), "aisle", "bearing;axis=long"), 90.0));
    let b2 = degrees(scene(), "b2", "bearing;axis=long");
    assert!(near(b2, 45.0), "{b2:?}");
}

#[test]
fn measured_angles_agree_with_the_wall_parallelism_judgement() {
    // `m` and `n`, judged a parallel pair, meet at 0°; `x` stands across `s`.
    let scene = || {
        storey()
            .relate("beside", "m", &["n", "s"])
            .relate("beside", "x", &["s"])
    };
    assert!(near(degrees(scene(), "m", "angle_to;path=beside"), 0.0));
    assert!(near(degrees(scene(), "x", "angle_to;path=beside"), 90.0));
    assert!(near(degrees(scene(), "x", "skew;path=beside"), 0.0));
    // A path reaching nothing measures nothing.
    let (project, services) = scene().services();
    assert!(matches!(
        measured_value(&services, &project, &id("n"), "angle_to;path=beside").unwrap(),
        PropertyResolution::Absent(_)
    ));
}

/// Each capability's verdicts reached by an expression over the values and
/// members measured as the capability measures them.
#[allow(clippy::needless_pass_by_value)]
mod as_expressions {
    use std::collections::BTreeSet;

    use axioval::engine::{
        CapabilityRegistry, PropertyRequest, PropertyResolutionError, PropertyResolutionService,
        PropertyResolutionServiceHandle,
    };
    use axioval::rules::ExpressionRequirement;
    use serde_json::{Value, json};

    use super::*;

    fn measured(name: &str) -> Value {
        json!({"kind": "property", "propertySet": "axioval:measured", "property": name})
    }

    fn field(name: &str) -> Value {
        json!({"kind": "property", "propertySet": "axioval:member", "property": name})
    }

    fn literal(value: f64, unit: &str) -> Value {
        json!({"kind": "literal", "value": {"type": "quantity", "value": value, "unit": unit}})
    }

    fn plain(value: f64) -> Value {
        json!({"kind": "literal", "value": {"type": "number", "value": value}})
    }

    /// `value` rounded to a micrometre (or square micrometre): a decimal
    /// bound then compares as its literal does.
    fn fine(value: Value, unit: &str) -> Value {
        json!({"kind": "round", "operand": value, "step": literal(1e-6, unit)})
    }

    fn compare(operator: &str, left: Value, right: Value) -> Value {
        json!({"kind": "compare", "operator": operator, "left": left, "right": right})
    }

    fn and(operands: Vec<Value>) -> Value {
        json!({"kind": "and", "operands": operands})
    }

    fn implies(antecedent: Value, consequent: Value) -> Value {
        json!({"kind": "implies", "antecedent": antecedent, "consequent": consequent})
    }

    fn over(function: &str, list: &str, value: Value) -> Value {
        json!({"kind": "aggregate", "function": function,
            "over": {"kind": "measured", "name": list}, "value": value})
    }

    /// The measured set answered through the registered measured values,
    /// as a run answers it.
    struct Measuring {
        services: ServiceRegistry,
        project: Project,
    }

    impl PropertyResolutionService for Measuring {
        fn resolve(
            &self,
            request: &PropertyRequest,
        ) -> Result<PropertyResolution, PropertyResolutionError> {
            if request.property_set() == Some(axioval::ir::MEASURED_SET) {
                return measured_value(
                    &self.services,
                    &self.project,
                    request.object_id(),
                    request.property(),
                );
            }
            Err(PropertyResolutionError::Unavailable(
                "this scene states no properties".into(),
            ))
        }
    }

    type Verdicts = (BTreeSet<String>, BTreeSet<String>);

    /// The flagged and the open objects of an evaluation.
    fn verdicts(evaluation: &CapabilityEvaluation) -> Verdicts {
        let local =
            |object: Option<&ObjectId>| object.map_or_else(String::new, |o| o.local_id.clone());
        (
            evaluation
                .findings()
                .iter()
                .map(|finding| local(finding.object_id()))
                .collect(),
            evaluation
                .not_evaluated_outcomes()
                .iter()
                .map(|outcome| local(outcome.object_id()))
                .collect(),
        )
    }

    /// The scene's services with the built-in measured values installed.
    fn installed(scene: Scene) -> (Project, ServiceRegistry) {
        let (project, mut services) = scene.services();
        axioval::rules::register_builtins(CapabilityRegistry::new())
            .unwrap()
            .install_measured(&mut services, &project);
        (project, services)
    }

    /// Evaluates `requirement` over the objects of `subjects` with the
    /// measured values installed.
    fn expression(scene: Scene, subjects: &str, requirement: &Value) -> CapabilityEvaluation {
        let rule = CompiledRule {
            id: RuleId::new("as-expression").unwrap(),
            capability: "axioval:capability.expression".into(),
            severity: Severity::Error,
            selector: kind(subjects),
            parameters: BTreeMap::from([(
                "requirement".to_owned(),
                ParameterValue::Expression {
                    value: Box::new(serde_json::from_value(requirement.clone()).unwrap()),
                },
            )]),
        };
        let (project, mut services) = installed(scene);
        let measuring = Measuring {
            services: services.clone(),
            project: project.clone(),
        };
        services
            .register(PropertyResolutionServiceHandle::new(Arc::new(measuring)))
            .unwrap();
        ExpressionRequirement.evaluate(
            &RuleContext {
                project: &project,
                services: &services,
            },
            &rule,
        )
    }

    /// Checks that `requirement` reaches the capability's verdicts on
    /// `scene` under `parameters`, and that those are `flagged` and `open`.
    fn parity(
        scene: fn() -> Scene,
        capability: (&dyn RuleCapability, &str),
        parameters: Vec<(&str, ParameterValue)>,
        requirement: &Value,
        (flagged, open): (&[&str], &[&str]),
    ) {
        let (check, subjects) = capability;
        let expected = verdicts(&scene().check(check, subjects, parameters));
        let set = |objects: &[&str]| -> BTreeSet<String> {
            objects.iter().map(|object| (*object).to_owned()).collect()
        };
        assert_eq!(expected, (set(flagged), set(open)), "{requirement}");
        let evaluation = expression(scene(), subjects, requirement);
        assert_eq!(
            verdicts(&evaluation),
            expected,
            "{requirement}: {:?}",
            evaluation.not_evaluated_outcomes()
        );
    }

    fn bay(
        scene: fn() -> Scene,
        parameters: Vec<(&str, ParameterValue)>,
        requirement: &Value,
        verdicts: (&[&str], &[&str]),
    ) {
        parity(
            scene,
            (&ParkingBay, "bay"),
            parameters,
            requirement,
            verdicts,
        );
    }

    fn side(side: &str) -> Value {
        fine(measured(&format!("rectangle_side;side={side}")), "m")
    }

    fn metre(value: f64) -> Value {
        literal(value, "m")
    }

    #[test]
    fn the_size_along_the_bay_s_own_axes_reaches_the_verdicts() {
        bay(
            car_park,
            vec![
                ("min_width", metres(2.4)),
                ("min_length", metres(5.0)),
                ("min_height", metres(2.1)),
            ],
            &and(vec![
                compare("greaterThanOrEquals", side("width"), metre(2.4)),
                compare("greaterThanOrEquals", side("length"), metre(5.0)),
                compare(
                    "greaterThanOrEquals",
                    fine(measured("extent_z"), "m"),
                    metre(2.1),
                ),
            ]),
            (&["b2"], &[]),
        );
        // A bound met exactly holds: both bays are 2.2 m high.
        bay(
            car_park,
            vec![
                ("max_width", metres(2.6)),
                ("max_length", metres(4.9)),
                ("max_height", metres(2.2)),
            ],
            &and(vec![
                compare("lessThanOrEquals", side("width"), metre(2.6)),
                compare("lessThanOrEquals", side("length"), metre(4.9)),
                compare(
                    "lessThanOrEquals",
                    fine(measured("extent_z"), "m"),
                    metre(2.2),
                ),
            ]),
            (&["b1"], &[]),
        );
    }

    const AISLES: &str = "axes_within;of=aisle";

    /// The test of `alignment` within 5 degrees on a member's angle.
    fn aligned(alignment: &str) -> Value {
        let angle = field("angle");
        match alignment {
            "parallel" => compare("lessThanOrEquals", angle, literal(5.0, "deg")),
            "perpendicular" => compare("greaterThanOrEquals", angle, literal(85.0, "deg")),
            _ => and(vec![
                compare("greaterThan", angle.clone(), literal(5.0, "deg")),
                compare("lessThan", angle, literal(85.0, "deg")),
            ]),
        }
    }

    fn orientation(alignment: &str) -> Vec<(&'static str, ParameterValue)> {
        vec![
            ("aisles", selector(kind("aisle"))),
            ("orientation", text(alignment)),
            ("angle_tolerance", quantity(5.0, "deg")),
        ]
    }

    /// A bay 0.5 m off its aisle.
    fn set_back() -> Scene {
        Scene::default()
            .body("aisle", "aisle", &rect(-5.0, 0.0, 25.0, 6.0), 0.0, 2.5)
            .body("b1", "bay", &rect(0.0, 6.5, 2.5, 11.5), 0.0, 2.2)
    }

    #[test]
    fn the_angle_to_the_aisles_within_reach_reaches_the_orientation_verdicts() {
        for (alignment, flagged) in [
            ("perpendicular", &["b2"][..]),
            ("angled", &["b1"]),
            ("parallel", &["b1", "b2"]),
        ] {
            bay(
                car_park,
                orientation(alignment),
                &over("any", AISLES, aligned(alignment)),
                (flagged, &[]),
            );
        }
        // An aisle out of reach is no aisle: none within 0.2 m, one within
        // 0.6 m.
        for (reach, flagged) in [(0.2, &["b1"][..]), (0.6, &[])] {
            let mut parameters = orientation("perpendicular");
            parameters.push(("aisle_reach", metres(reach)));
            bay(
                set_back,
                parameters,
                &over(
                    "any",
                    &format!("{AISLES};reach={reach}"),
                    aligned("perpendicular"),
                ),
                (flagged, &[]),
            );
        }
    }

    fn count(at: &str, reach: f64, zone: Option<f64>) -> Value {
        let zone = zone.map_or_else(String::new, |zone| format!(";side_zone={zone}"));
        measured(&format!(
            "obstruction_count;obstacles=column;reach={reach};at={at}{zone}"
        ))
    }

    /// At most `ends` ends and `sides` sides obstructed, nothing within.
    fn obstructed(reach: f64, zone: Option<f64>, (ends, sides): (f64, f64)) -> Value {
        and(vec![
            compare("lessThanOrEquals", count("within", reach, zone), plain(0.0)),
            compare("lessThanOrEquals", count("ends", reach, zone), plain(ends)),
            compare(
                "lessThanOrEquals",
                count("sides", reach, zone),
                plain(sides),
            ),
        ])
    }

    fn obstructions(reach: f64, ends: &str, sides: &str) -> Vec<(&'static str, ParameterValue)> {
        vec![
            ("obstacles", selector(kind("column"))),
            ("obstruction_reach", metres(reach)),
            ("end_obstructions", text(ends)),
            ("side_obstructions", text(sides)),
        ]
    }

    /// The car park with a column standing within `b1`.
    fn column_within() -> Scene {
        car_park().body("c4", "column", &rect(1.0, 8.0, 1.4, 8.4), 0.0, 3.0)
    }

    /// A 3 m square bay with a column past one of its edges.
    fn square() -> Scene {
        Scene::default()
            .body("aisle", "aisle", &rect(-5.0, 0.0, 25.0, 6.0), 0.0, 2.5)
            .body("sq", "bay", &rect(0.0, 6.0, 3.0, 9.0), 0.0, 2.2)
            .body("c1", "column", &rect(1.0, 9.05, 1.4, 9.45), 0.0, 3.0)
    }

    #[test]
    fn the_obstructed_ends_and_sides_reach_the_verdicts() {
        for (reach, (ends, sides), allowed, flagged) in [
            (0.2, ("none", "one"), (0.0, 1.0), &["b1"][..]),
            (0.2, ("one", "none"), (1.0, 0.0), &["b1"]),
            (0.2, ("one", "one"), (1.0, 1.0), &[]),
            (0.08, ("none", "none"), (0.0, 0.0), &["b1"]),
        ] {
            bay(
                car_park,
                obstructions(reach, ends, sides),
                &obstructed(reach, None, allowed),
                (flagged, &[]),
            );
        }
        // A column standing within a bay is a finding, whatever is allowed.
        bay(
            column_within,
            obstructions(0.2, "both", "both"),
            &obstructed(0.2, None, (2.0, 2.0)),
            (&["b1"], &[]),
        );
        // A square bay has no ends: its width is judged, its edges are not.
        let mut parameters = obstructions(0.2, "none", "both");
        parameters.push(("min_width", metres(2.5)));
        bay(
            square,
            parameters,
            &and(vec![
                compare("greaterThanOrEquals", side("width"), metre(2.5)),
                obstructed(0.2, None, (0.0, 2.0)),
            ]),
            (&[], &["sq"]),
        );
    }

    #[test]
    fn the_side_zone_reaches_the_verdicts() {
        for (zone, flagged) in [(3.0, &[][..]), (4.8, &["p1"])] {
            let mut parameters = obstructions(0.1, "both", "none");
            parameters.push(("side_zone_length", metres(zone)));
            bay(
                mixed_bays,
                parameters,
                &obstructed(0.1, Some(zone), (2.0, 0.0)),
                (flagged, &[]),
            );
        }
    }

    #[test]
    fn size_bounds_filtered_by_state_reach_the_verdicts() {
        // Orientation: the bound applies to bays at that angle to every
        // aisle they meet.
        for (alignment, flagged) in [
            ("perpendicular", &["p1", "p2", "p3"][..]),
            ("parallel", &[]),
        ] {
            bay(
                mixed_bays,
                vec![
                    ("min_length", metres(5.0)),
                    ("applies_when", text("filter")),
                    ("orientations", path(&[alignment])),
                    ("aisles", selector(kind("aisle"))),
                    ("angle_tolerance", quantity(5.0, "deg")),
                ],
                &implies(
                    over("all", AISLES, aligned(alignment)),
                    compare("greaterThanOrEquals", side("length"), metre(5.0)),
                ),
                (flagged, &[]),
            );
        }
        // Obstructed sides: the bound applies to bays with none obstructed.
        for (zone, flagged) in [
            (None, &["p2", "p3", "q"][..]),
            (Some(3.0), &["p1", "p2", "p3", "q"]),
        ] {
            let mut parameters = vec![
                ("min_width", metres(2.6)),
                ("applies_when", text("filter")),
                ("side_states", path(&["none"])),
                ("obstacles", selector(kind("column"))),
                ("obstruction_reach", metres(0.1)),
            ];
            if let Some(zone) = zone {
                parameters.push(("side_zone_length", metres(zone)));
            }
            bay(
                mixed_bays,
                parameters,
                &and(vec![
                    compare("lessThanOrEquals", count("within", 0.1, zone), plain(0.0)),
                    implies(
                        compare("equals", count("sides", 0.1, zone), plain(0.0)),
                        compare("greaterThanOrEquals", side("width"), metre(2.6)),
                    ),
                ]),
                (flagged, &[]),
            );
        }
    }

    fn spaced(extra: Vec<(&'static str, ParameterValue)>, requirement: &Value, flagged: &[&str]) {
        parity(
            storey,
            (&WallSpacing, "storey"),
            spacing(extra),
            requirement,
            (flagged, &[]),
        );
    }

    const MEMBERS: &str = "members=wall;member_path=contains;angle_tolerance=5";

    #[test]
    fn the_parallel_pairs_reach_the_minimum_spacing_verdicts() {
        for (minimum, flagged) in [(1.0, &["st"][..]), (0.5, &["st"]), (0.3, &[])] {
            let pairs = format!("parallel_pairs;{MEMBERS};reach={minimum}");
            spaced(
                vec![("minimum", metres(minimum))],
                &over(
                    "none",
                    &pairs,
                    compare("lessThan", fine(field("distance"), "m"), metre(minimum)),
                ),
                flagged,
            );
        }
        // Within 6 m: `m` and `n`, `s` and `m`, `s` and `n`; `x` stands
        // across them.
        let (_, services) = installed(storey());
        let pairs = axioval::engine::measured_members(
            &services,
            &id("st"),
            &format!("parallel_pairs;{MEMBERS};reach=6"),
        )
        .unwrap();
        assert_eq!(pairs.len(), 3, "{pairs:#?}");
        assert!(pairs.iter().all(|pair| pair.certain));
    }

    #[test]
    fn the_area_outside_the_bands_reaches_the_coverage_verdicts() {
        for (maximum, above, flagged) in [
            (6.0, 1.0, &["st"][..]),
            (6.0, 23.0, &[]),
            (5.0, 1.0, &["st"]),
            (5.0, 34.0, &[]),
        ] {
            let area = measured(&format!(
                "band_uncovered_area;{MEMBERS};maximum={maximum};footprints=slab;\
                 footprint_path=contains"
            ));
            spaced(
                vec![
                    ("maximum", metres(maximum)),
                    ("footprints", selector(kind("slab"))),
                    ("footprint_path", path(&[CONTAINS])),
                    ("uncovered_above", quantity(above, "m2")),
                ],
                &compare("lessThanOrEquals", fine(area, "m2"), literal(above, "m2")),
                flagged,
            );
        }
    }
}
