//! Values measured from geometry, read as the reserved `axioval:measured`
//! property set by selectors and comparisons: intervals sure to hold the
//! value, undecided when they straddle a bound.
#![allow(missing_docs, clippy::float_cmp)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, ElevationInterval, EvidenceSession, VerticalExtent, VerticalExtentError,
    VerticalExtentService, VerticalExtentServiceHandle,
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
