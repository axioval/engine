//! Values measured from geometry, read as the reserved `axioval:measured`
//! property set by selectors and comparisons: intervals sure to hold the
//! value, undecided when they straddle a bound.
#![allow(missing_docs, clippy::float_cmp)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    BoundaryCoverage, BoundaryCoverageError, BoundaryCoverageRequest, BoundaryCoverageService,
    BoundaryCoverageServiceHandle, BoundaryPlacement, CapabilityRegistry, CoverageAreas,
    ElevationInterval, EngineError, EvidenceSession, MeasuredBoundary, MetricDirection,
    MetricFrame, MetricPoint, ObjectFrame, ObjectFrameError, ObjectFrameService,
    ObjectFrameServiceHandle, ObjectFront, SurfaceAreaInterval, VerticalExtent,
    VerticalExtentError, VerticalExtentService, VerticalExtentServiceHandle,
};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, Report, Scope};
use axioval_rules::register_builtins;
use common::runtime::{definitions, entity, plan, rule, run, session, snapshot};
use common::{Model, id, source};
use serde_json::{Value, json};

const EXISTS: &str = "axioval:capability.property-exists";
const COMPARISON: &str = "axioval:capability.property-comparison";

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

/// Bottom and top elevations per object, each an interval.
struct Extents(BTreeMap<ObjectId, [(f64, f64); 2]>);

impl VerticalExtentService for Extents {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        let [bottom, top] = self
            .0
            .get(object)
            .copied()
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?;
        let exact = bottom.0 == bottom.1 && top.0 == top.1;
        let mut evidence = Evidence::exact(source(), format!("extent:{object}"));
        evidence.exact = exact;
        VerticalExtent::try_new(
            object.clone(),
            ElevationInterval::try_new(bottom.0, bottom.1)?,
            ElevationInterval::try_new(top.0, top.1)?,
            evidence,
        )
    }
}

fn with_extents(model: Model, extents: &[(&str, [(f64, f64); 2])]) -> EvidenceSession {
    let extents = Extents(
        extents
            .iter()
            .map(|(local, extent)| (id(local), *extent))
            .collect(),
    );
    session(model)
        .with_host_service(
            VerticalExtentServiceHandle::new(Arc::new(extents)),
            &[snapshot()],
        )
        .unwrap()
}

fn subjects(report: &Report, rule: &str) -> Vec<String> {
    report
        .findings()
        .iter()
        .filter(|finding| finding.rule_id.to_string() == rule)
        .map(common::subject)
        .collect()
}

fn open(report: &Report, rule: &str) -> Vec<(Scope, NotEvaluatedReason)> {
    report
        .not_evaluated
        .iter()
        .filter(|outcome| outcome.rule_id.to_string() == rule)
        .map(|outcome| (outcome.scope.clone(), outcome.reason.clone()))
        .collect()
}

/// Pipes lying flat: 40 mm, 100 mm, and a tessellated one between 45 and
/// 55 mm.
/// Bottom and top elevation intervals of named objects.
type Stated = Vec<(&'static str, [(f64, f64); 2])>;

fn pipes() -> (Model, Stated) {
    let model = Model::default()
        .object("p40", "pipe")
        .object("p100", "pipe")
        .object("p50", "pipe");
    let extents = vec![
        ("p40", [(1.0, 1.0), (1.04, 1.04)]),
        ("p100", [(1.0, 1.0), (1.1, 1.1)]),
        ("p50", [(0.9975, 1.0025), (1.0475, 1.0525)]),
    ];
    (model, extents)
}

/// Every small pipe (`extent_z < 0.05 m`) has a label, which none has.
fn small_pipes_labelled() -> Value {
    rule(
        "small-pipes",
        EXISTS,
        "error",
        json!({ "kind": "allOf", "operands": [
            entity("pipe"),
            { "kind": "property", "propertySet": "axioval:measured", "property": "extent_z",
              "operator": "lessThan", "value": { "type": "quantity", "value": 0.05, "unit": "m" } },
        ] }),
        json!({ "property": { "type": "propertyReference", "property": "t.Label" } }),
        json!({}),
    )
}

fn check(rules: Vec<Value>, session: &EvidenceSession, types: &[&str]) -> Report {
    let registry = registry();
    let definitions = definitions(&registry, &[EXISTS, COMPARISON], types, &["Label"], &[]);
    let plan = plan(&registry, &definitions, rules).unwrap();
    run(registry, plan, session, |runtime| runtime).unwrap()
}

#[test]
fn a_measured_extent_selects_small_pipes_and_leaves_a_straddling_one_open() {
    let (model, extents) = pipes();
    let report = check(
        vec![small_pipes_labelled()],
        &with_extents(model, &extents),
        &["pipe"],
    );
    assert_eq!(subjects(&report, "small-pipes"), ["p40"]);
    assert_eq!(
        open(&report, "small-pipes"),
        [(
            Scope::Object(id("p50")),
            NotEvaluatedReason::IncompleteEvidence
        )]
    );
}

#[test]
fn without_geometry_a_rule_reports_one_missing_service() {
    let (model, _) = pipes();
    let report = check(vec![small_pipes_labelled()], &session(model), &["pipe"]);
    assert!(subjects(&report, "small-pipes").is_empty());
    assert_eq!(
        open(&report, "small-pipes"),
        [(Scope::Source(source()), NotEvaluatedReason::MissingService)]
    );
}

#[test]
fn a_door_bottom_is_compared_with_its_space_bottom() {
    // Each space contains one door; the space's floor is at 0.
    let model = Model::default()
        .object("s1", "space")
        .object("s2", "space")
        .object("s3", "space")
        .object("level", "door")
        .object("raised", "door")
        .object("meshed", "door")
        .edge("contains", "s1", "level")
        .edge("contains", "s2", "raised")
        .edge("contains", "s3", "meshed");
    let floor = [(0.0, 0.0), (3.0, 3.0)];
    let extents = [
        ("s1", floor),
        ("s2", floor),
        ("s3", floor),
        ("level", [(0.0, 0.0), (2.1, 2.1)]),
        ("raised", [(0.2, 0.2), (2.3, 2.3)]),
        ("meshed", [(-0.01, 0.01), (2.09, 2.11)]),
    ];
    let comparison = rule(
        "door-floor",
        COMPARISON,
        "error",
        entity("space"),
        json!({
            "compared_selector": { "type": "selector", "value": entity("door") },
            "compared_property": { "type": "propertyReference",
                                   "propertySet": "axioval:measured", "property": "bottom" },
            "target_property": { "type": "propertyReference",
                                 "propertySet": "axioval:measured", "property": "bottom" },
            "operator": { "type": "string", "value": "equals" },
            "factor": { "type": "number", "value": 1.0 },
            "component_mode": { "type": "string", "value": "related" },
            "relationship": { "type": "string", "value": "contains" },
            "quantifier": { "type": "string", "value": "each" },
        }),
        json!({}),
    );
    let report = check(
        vec![comparison],
        &with_extents(model, &extents),
        &["space", "door"],
    );
    assert_eq!(subjects(&report, "door-floor"), ["s2"]);
    assert_eq!(
        open(&report, "door-floor"),
        [(
            Scope::Object(id("s3")),
            NotEvaluatedReason::IncompleteEvidence
        )]
    );
}

/// Placement origins per object, axis-aligned.
struct Origins(
    BTreeMap<ObjectId, [f64; 3]>,
    Vec<axioval_engine::SourceSnapshot>,
);

impl ObjectFrameService for Origins {
    fn source_snapshots(&self) -> &[axioval_engine::SourceSnapshot] {
        &self.1
    }

    fn object_frame(&self, object: &ObjectId) -> Result<ObjectFrame, ObjectFrameError> {
        let origin = *self
            .0
            .get(object)
            .ok_or_else(|| ObjectFrameError::Unreadable("unplaced".into()))?;
        let axis = |vector| MetricDirection::try_new(vector).unwrap();
        let frame = MetricFrame::try_new(
            MetricPoint::try_new(object.clone(), origin).unwrap(),
            axis([1.0, 0.0, 0.0]),
            axis([0.0, 1.0, 0.0]),
            axis([0.0, 0.0, 1.0]),
        )
        .unwrap();
        ObjectFrame::try_new(
            object.clone(),
            frame,
            ObjectFront::NotStated,
            Evidence::exact(source(), format!("placement:{object}")),
        )
    }
}

/// Every small pipe (by `name` against `bound`) has a label, which none has.
fn labelled_where(name: &str, operator: &str, bound: f64, unit: &str, kind: &str) -> Value {
    rule(
        "measured",
        EXISTS,
        "error",
        json!({ "kind": "allOf", "operands": [
            entity(kind),
            { "kind": "property", "propertySet": "axioval:measured", "property": name,
              "operator": operator, "value": { "type": "quantity", "value": bound, "unit": unit } },
        ] }),
        json!({ "property": { "type": "propertyReference", "property": "t.Label" } }),
        json!({}),
    )
}

#[test]
fn a_placement_coordinate_selects_objects_above_a_height() {
    let model = Model::default()
        .object("low", "pipe")
        .object("high", "pipe");
    let origins = Origins(
        BTreeMap::from([(id("low"), [1.0, 2.0, 0.5]), (id("high"), [1.0, 2.0, 3.5])]),
        vec![snapshot()],
    );
    let session = session(model)
        .with_host_service(
            ObjectFrameServiceHandle::new(Arc::new(origins)),
            &[snapshot()],
        )
        .unwrap();
    let report = check(
        vec![labelled_where("z", "greaterThan", 3.0, "m", "pipe")],
        &session,
        &["pipe"],
    );
    assert_eq!(subjects(&report, "measured"), ["high"]);
}

#[test]
fn a_bottom_is_measured_above_the_level_the_path_reaches() {
    // Storey at 3 m holds a sill 0.9 m above it and one 1.2 m above it; a
    // third window is in no storey.
    let model = Model::default()
        .object("storey", "storey")
        .object("w1", "window")
        .object("w2", "window")
        .object("loose", "window")
        .edge("contains", "storey", "w1")
        .edge("contains", "storey", "w2");
    let origins = Origins(
        BTreeMap::from([(id("storey"), [0.0, 0.0, 3.0])]),
        vec![snapshot()],
    );
    let extents = [
        ("w1", [(3.9, 3.9), (5.0, 5.0)]),
        ("w2", [(4.2, 4.2), (5.0, 5.0)]),
        ("loose", [(0.0, 0.0), (1.0, 1.0)]),
    ];
    let session = with_extents(model, &extents)
        .with_host_service(
            ObjectFrameServiceHandle::new(Arc::new(origins)),
            &[snapshot()],
        )
        .unwrap();
    let report = check(
        vec![labelled_where(
            "bottom_above_level;path=contains:backward",
            "lessThan",
            1.0,
            "m",
            "window",
        )],
        &session,
        &["window"],
    );
    // Only w1 sits below 1 m; the loose window has no level, so no value.
    assert_eq!(subjects(&report, "measured"), ["w1"]);
    assert!(open(&report, "measured").is_empty());
}

/// Kinds without subtypes.
struct Flat(Vec<axioval_engine::SourceSnapshot>);

impl axioval_engine::TypeHierarchyService for Flat {
    fn source_snapshots(&self) -> &[axioval_engine::SourceSnapshot] {
        &self.0
    }

    fn is_a(&self, kind: &str, ancestor: &str) -> Result<bool, axioval_engine::TypeHierarchyError> {
        Ok(kind.eq_ignore_ascii_case(ancestor))
    }
}

/// A space bounded by a wall (10 m²) and a door (2 m²); the other space
/// has a boundary naming no element.
struct Boundaries;

impl BoundaryCoverageService for Boundaries {
    fn measure_boundary_coverage(
        &self,
        request: &BoundaryCoverageRequest,
    ) -> Result<BoundaryCoverage, BoundaryCoverageError> {
        let area = |value| SurfaceAreaInterval::exact(value).unwrap();
        let on = |value| BoundaryPlacement::OnSurface { area: area(value) };
        let boundaries = if *request.space() == id("s1") {
            vec![
                MeasuredBoundary::new(id("b1"), Some(id("wall")), on(10.0)),
                MeasuredBoundary::new(id("b2"), Some(id("door")), on(2.0)),
            ]
        } else {
            vec![MeasuredBoundary::new(id("b3"), None, on(12.0))]
        };
        BoundaryCoverage::try_new(
            request.clone(),
            CoverageAreas {
                surface: area(12.0),
                covered: area(12.0),
                uncovered: area(0.0),
                overlap: area(0.0),
            },
            boundaries,
            Vec::new(),
            Evidence::exact(source(), format!("boundaries:{}", request.space())),
        )
    }
}

#[test]
fn boundary_areas_are_summed_per_kind_of_bounding_element() {
    let model = Model::default()
        .object("s1", "space")
        .object("s2", "space")
        .object("wall", "wall")
        .object("door", "door");
    let session = session(model)
        .with_host_service(
            BoundaryCoverageServiceHandle::new(Arc::new(Boundaries)),
            &[snapshot()],
        )
        .unwrap()
        .with_service(axioval_engine::TypeHierarchyServiceHandle::new(Arc::new(
            Flat(vec![snapshot()]),
        )))
        .unwrap();
    let report = check(
        vec![labelled_where(
            "boundary_area;kind=wall",
            "greaterThan",
            5.0,
            "m2",
            "space",
        )],
        &session,
        &["space"],
    );
    assert_eq!(subjects(&report, "measured"), ["s1"]);
    // A boundary naming no element may be a wall.
    assert_eq!(
        open(&report, "measured"),
        [(
            Scope::Object(id("s2")),
            NotEvaluatedReason::BackendUnavailable
        )]
    );
}

#[test]
fn an_unknown_or_malformed_measured_name_is_refused() {
    let registry = registry();
    let definitions = definitions(&registry, &[EXISTS], &["pipe"], &["Label"], &[]);
    for name in ["height", "bottom_above_level", "boundary_area;plane=1"] {
        assert!(
            matches!(
                plan(
                    &registry,
                    &definitions,
                    vec![labelled_where(name, "lessThan", 1.0, "m", "pipe")]
                ),
                Err(EngineError::UnknownConcept { .. })
            ),
            "{name}"
        );
    }
}
