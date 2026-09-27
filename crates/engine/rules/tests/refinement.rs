//! What a rule instance declares about its outcomes beyond the capability's
//! verdicts, run end to end: compiled from packages and executed by the
//! runtime over a session.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, EngineError, EvidenceSession, PlanArea, PlanAreaError, PlanAreaService,
    PlanAreaServiceHandle,
};
use axioval_ir::{Evidence, ObjectId, Report, Severity};
use axioval_rules::register_builtins;
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
