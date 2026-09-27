//! What a rule instance declares about its outcomes beyond the capability's
//! verdicts, run end to end: compiled from packages and executed by the
//! runtime over a session.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    Bounds3, CapabilityRegistry, EngineError, EvidenceSession, GeometryFidelity, LengthInterval,
    ObjectBounds, PlanArea, PlanAreaError, PlanAreaService, PlanAreaServiceHandle, ProximityError,
    ProximityEvidence, ProximityRequest, ProximityService, ProximityServiceHandle,
};
use axioval_ir::{Evidence, ObjectId, Report, Severity};
use axioval_rules::{Clash, register_builtins};
use common::runtime::{definitions, entity, plan, rule, run, session, snapshot};
use common::{Model, id, source};
use serde_json::{Value, json};

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

/// `(subject, severity)` of every finding, in report order.
fn graded(report: &Report) -> Vec<(String, Severity)> {
    report
        .findings()
        .iter()
        .map(|finding| (common::subject(finding), finding.severity.clone()))
        .collect()
}

/// Footprint areas as stated intervals.
struct Areas(BTreeMap<ObjectId, (f64, f64)>);

impl PlanAreaService for Areas {
    #[allow(clippy::float_cmp)]
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        let (lower, upper) = self
            .0
            .get(object)
            .copied()
            .ok_or_else(|| PlanAreaError::UnknownObject(object.clone()))?;
        let mut evidence = Evidence::exact(source(), format!("footprint:{object}"));
        evidence.exact = lower == upper;
        PlanArea::try_new(lower, upper, evidence)
    }

    fn measure_plan_overlap(
        &self,
        first: &ObjectId,
        _: &ObjectId,
    ) -> Result<PlanArea, PlanAreaError> {
        Err(PlanAreaError::UnknownObject(first.clone()))
    }
}

mod severity_bands {
    use super::*;

    const PLAN_AREA: &str = "axioval:capability.plan-area";

    /// Spaces of 97, 70 and between 92 and 97 m², against a 100 m² minimum:
    /// 3 %, 30 % and 3 to 8 % short.
    fn spaces() -> EvidenceSession {
        let model = Model::default()
            .object("small", "space")
            .object("tiny", "space")
            .object("unsure", "space");
        let areas = Areas(BTreeMap::from([
            (id("small"), (97.0, 97.0)),
            (id("tiny"), (70.0, 70.0)),
            (id("unsure"), (92.0, 97.0)),
        ]));
        session(model)
            .with_host_service(PlanAreaServiceHandle::new(Arc::new(areas)), &[snapshot()])
            .unwrap()
    }

    fn area_rule(extra: Value) -> Value {
        rule(
            "area",
            PLAN_AREA,
            "error",
            entity("space"),
            json!({ "minimum": { "type": "number", "value": 100.0 } }),
            extra,
        )
    }

    fn bands() -> Value {
        json!({ "severityBands": [
            { "below": 0.05, "severity": "info" },
            { "below": 0.2, "severity": "warning" },
        ] })
    }

    fn check(extra: Value) -> Report {
        let registry = registry();
        let definitions = definitions(&registry, &[PLAN_AREA], &["space"], &[], &[]);
        let plan = plan(&registry, &definitions, vec![area_rule(extra)]).unwrap();
        run(registry, plan, &spaces(), |runtime| runtime).unwrap()
    }

    #[test]
    fn a_shortfall_is_graded_by_its_band() {
        let report = check(bands());
        assert_eq!(
            graded(&report),
            [
                ("small".into(), Severity::Info),
                ("tiny".into(), Severity::Error),
                ("unsure".into(), Severity::Warning),
            ]
        );
    }

    #[test]
    fn a_deviation_straddling_bands_takes_the_worst_and_says_so() {
        let report = check(bands());
        let unsure = &report.findings()[2];
        assert!(
            unsure
                .message
                .ends_with("graded warning by its most severe band"),
            "{}",
            unsure.message
        );
        // A deviation within one band keeps its message.
        assert!(!report.findings()[0].message.contains("graded"));
    }

    #[test]
    fn without_bands_every_finding_keeps_the_rule_severity() {
        let banded = check(bands());
        let plain = check(json!({}));
        assert!(
            graded(&plain)
                .iter()
                .all(|(_, severity)| *severity == Severity::Error)
        );
        // Grading changes the severity, never which findings exist.
        assert_eq!(banded.findings().len(), plain.findings().len());
    }

    #[test]
    fn bands_on_a_capability_reporting_no_deviation_are_refused() {
        let registry = registry();
        let capability = "axioval:capability.property-exists";
        let definitions = definitions(&registry, &[capability], &["space"], &["Name"], &[]);
        let rule = rule(
            "exists",
            capability,
            "error",
            entity("space"),
            json!({ "property": { "type": "propertyReference", "property": "t.Name" } }),
            bands(),
        );
        assert!(matches!(
            plan(&registry, &definitions, vec![rule]),
            Err(EngineError::InvalidRefinement { .. })
        ));
    }

    #[test]
    fn bands_must_ascend() {
        let registry = registry();
        let definitions = definitions(&registry, &[PLAN_AREA], &["space"], &[], &[]);
        let descending = area_rule(json!({ "severityBands": [
            { "below": 0.2, "severity": "warning" },
            { "below": 0.05, "severity": "info" },
        ] }));
        assert!(matches!(
            plan(&registry, &definitions, vec![descending]),
            Err(EngineError::InvalidRefinement { .. })
        ));
    }
}

/// Unit boxes along x: every pair overlapping in x clashes by its overlap.
struct Boxes(BTreeMap<ObjectId, f64>);

impl ProximityService for Boxes {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        let x = *self.0.get(object).ok_or(ProximityError::Unavailable)?;
        ObjectBounds::try_new(
            object.clone(),
            Bounds3::try_new([x, 0.0, 0.0], [x + 1.0, 1.0, 1.0])?,
            GeometryFidelity::Exact,
        )
    }

    fn measure_proximity(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProximityEvidence, ProximityError> {
        let gap = (self.0[request.subject()] - self.0[request.counterpart()]).abs() - 1.0;
        ProximityEvidence::try_new(
            request.clone(),
            gap.max(0.0),
            Some((-gap).max(0.0)),
            0.0,
            None,
            GeometryFidelity::Exact,
            Evidence::exact(
                source(),
                format!("proximity:{}:{}", request.subject(), request.counterpart()),
            ),
        )?
        .with_hausdorff(LengthInterval::try_new(1.0, 1.0).unwrap())
    }
}

mod severity_overrides {
    use super::*;

    const CLASH: &str = "axioval:capability.clash";

    /// Ducts `d1`, `d2` and `d3` each run through one wall: `w1` is load
    /// bearing, `w2` is not, and whether `w3` is cannot be read.
    fn ducts_and_walls() -> EvidenceSession {
        let model = Model::default()
            .object("d1", "duct")
            .object("d2", "duct")
            .object("d3", "duct")
            .object("w1", "wall")
            .object("w2", "wall")
            .object("w3", "wall")
            .text("w1", "Pset", "LoadBearing", "yes")
            .text("w2", "Pset", "LoadBearing", "no")
            .unreadable("w3");
        let boxes = Boxes(BTreeMap::from([
            (id("d1"), 0.0),
            (id("w1"), 0.5),
            (id("d2"), 5.0),
            (id("w2"), 5.5),
            (id("d3"), 10.0),
            (id("w3"), 10.5),
        ]));
        session(model)
            .with_host_service(ProximityServiceHandle::new(Arc::new(boxes)), &[snapshot()])
            .unwrap()
    }

    fn load_bearing() -> Value {
        json!({
            "kind": "property", "propertySet": "t.Pset", "property": "t.LoadBearing",
            "operator": "equals", "value": {"type": "string", "value": "yes"},
        })
    }

    fn check(extra: Value) -> Result<Report, EngineError> {
        let registry = registry();
        let definitions = definitions(
            &registry,
            &[CLASH],
            &["duct", "wall"],
            &["LoadBearing"],
            &["Pset"],
        );
        let rule = rule(
            "ducts-through-walls",
            CLASH,
            "warning",
            entity("duct"),
            json!({ "counterparts": { "type": "selector", "value": entity("wall") },
                "penetration_tolerance_metres": { "type": "number", "value": 0.01 } }),
            extra,
        );
        let plan = plan(&registry, &definitions, vec![rule])?;
        run(registry, plan, &ducts_and_walls(), |runtime| runtime)
    }

    #[test]
    fn a_clash_with_a_load_bearing_wall_is_an_error() {
        let report = check(json!({ "severityOverrides": [
            { "selector": load_bearing(), "severity": "error" },
        ] }))
        .unwrap();
        assert_eq!(
            graded(&report),
            [
                ("d1".into(), Severity::Error),
                ("d2".into(), Severity::Warning),
            ]
        );
        // The wall's load bearing is cited beside the clash.
        assert!(
            report.findings()[0]
                .evidence
                .iter()
                .any(|evidence| evidence.locator.contains("LoadBearing"))
        );
        // Whether `w3` bears load is unknown: the clash stands, its severity
        // does not, and it is never defaulted to the rule's.
        let [undecided] = report.not_evaluated() else {
            panic!("{report:?}");
        };
        assert_eq!(undecided.object_id(), Some(&id("d3")));
        assert!(
            undecided.message.contains("error or warning"),
            "{}",
            undecided.message
        );
    }

    #[test]
    fn without_overrides_every_clash_keeps_the_rule_severity() {
        let report = check(json!({})).unwrap();
        assert_eq!(
            graded(&report),
            [
                ("d1".into(), Severity::Warning),
                ("d2".into(), Severity::Warning),
                ("d3".into(), Severity::Warning),
            ]
        );
    }

    #[test]
    fn an_undecided_override_that_cannot_change_the_severity_decides_nothing() {
        // Whether or not `w3` bears load, `d3` is a warning.
        let report = check(json!({ "severityOverrides": [
            { "selector": load_bearing(), "severity": "warning" },
        ] }))
        .unwrap();
        assert_eq!(
            graded(&report),
            [
                ("d1".into(), Severity::Warning),
                ("d2".into(), Severity::Warning),
                ("d3".into(), Severity::Warning),
            ]
        );
        assert!(report.not_evaluated().is_empty());
    }

    #[test]
    fn overrides_need_a_refiner_and_known_concepts() {
        let registry = CapabilityRegistry::new().register(Clash).unwrap();
        let definitions = definitions(&registry, &[CLASH], &["duct", "wall"], &[], &[]);
        let overriding = |selector: Value| {
            rule(
                "r",
                CLASH,
                "warning",
                entity("duct"),
                json!({ "counterparts": { "type": "selector", "value": entity("wall") },
                "penetration_tolerance_metres": { "type": "number", "value": 0.01 } }),
                json!({ "severityOverrides": [{ "selector": selector, "severity": "error" }] }),
            )
        };
        assert!(matches!(
            plan(&registry, &definitions, vec![overriding(entity("wall"))]),
            Err(EngineError::InvalidRefinement { .. })
        ));
        let registry = super::registry();
        assert!(matches!(
            plan(&registry, &definitions, vec![overriding(entity("column"))]),
            Err(EngineError::UnknownConcept { .. })
        ));
    }
}
