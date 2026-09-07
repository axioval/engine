//! Space-validation capability contract tests.
//!
//! ADR 0004: the service measures, the capability decides. These pin each
//! aspect independently -- in particular that one unavailable aspect does not
//! sink the others, which is the defect the bundled source fact struct had.
#![allow(missing_docs)]

use std::{collections::BTreeMap, sync::Arc};

use axioval_engine::{
    BoundaryGap, Cap, CapCoverage, ClearHeightEvidence, CompiledRule, Containment,
    NotEvaluatedReason, RuleCapability, RuleContext, ServiceRegistry, SpaceError, SpaceOverlap,
    SpaceService, SpaceServiceHandle, StoreyResidual, SupportCounts,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity as RuleSeverity};
use axioval_ir::{Evidence, Object, ObjectId, Project, RuleId, Severity, SourceId};
use axioval_rules::SpaceValidation;

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}
fn oid(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

fn rule_with(overrides: &[(&str, ParameterValue)]) -> CompiledRule {
    let mut parameters = BTreeMap::from([
        (
            "required_height_metres".to_string(),
            ParameterValue::Number { value: 2.5 },
        ),
        (
            "uncovered_segment_length_metres".to_string(),
            ParameterValue::Number { value: 1.0 },
        ),
        (
            "check_top_cap".to_string(),
            ParameterValue::Boolean { value: false },
        ),
        (
            "check_bottom_cap".to_string(),
            ParameterValue::Boolean { value: false },
        ),
        (
            "check_unallocated_area".to_string(),
            ParameterValue::Boolean { value: false },
        ),
        (
            "maximum_unallocated_area_square_metres".to_string(),
            ParameterValue::Number { value: 1.0 },
        ),
    ]);
    for (key, value) in overrides {
        parameters.insert((*key).to_string(), value.clone());
    }
    CompiledRule {
        id: RuleId::new("space-validation").unwrap(),
        capability: "axioval:capability.space-validation".into(),
        severity: RuleSeverity::Warning,
        selector: Selector::EntityType {
            object_type: "space".into(),
            include_subtypes: true,
        },
        parameters,
    }
}
fn rule() -> CompiledRule {
    rule_with(&[])
}

/// Every aspect answers independently, so a test can make exactly one fail.
/// One stubbed answer per aspect.
type Answer<T> = Option<Result<T, SpaceError>>;
type GapRows = Vec<(f64, Vec<ObjectId>)>;
type OverlapRows = Vec<(bool, f64, f64, Containment)>;
type ResidualRows = Vec<(ObjectId, f64)>;

#[derive(Default)]
struct Stub {
    duplicates: Answer<Vec<ObjectId>>,
    height: Answer<f64>,
    gaps: Answer<GapRows>,
    overlaps: Answer<OverlapRows>,
    cap: Answer<(f64, f64)>,
    residuals: Answer<ResidualRows>,
    support: Answer<(usize, usize)>,
}

fn evidence() -> Evidence {
    Evidence::exact(source(), "space:measured")
}

impl SpaceService for Stub {
    fn measure_duplicates(&self, _space: &ObjectId) -> Result<Vec<ObjectId>, SpaceError> {
        self.duplicates.clone().unwrap_or(Ok(Vec::new()))
    }
    fn measure_clear_height(&self, space: &ObjectId) -> Result<ClearHeightEvidence, SpaceError> {
        let metres = self.height.unwrap_or(Ok(3.0))?;
        ClearHeightEvidence::try_new(space.clone(), metres, evidence())
    }
    fn measure_boundary_gaps(&self, _space: &ObjectId) -> Result<Vec<BoundaryGap>, SpaceError> {
        self.gaps
            .clone()
            .unwrap_or(Ok(Vec::new()))?
            .into_iter()
            .map(|(length, elements)| BoundaryGap::try_new(length, elements))
            .collect()
    }
    fn measure_overlaps(&self, _space: &ObjectId) -> Result<Vec<SpaceOverlap>, SpaceError> {
        self.overlaps
            .clone()
            .unwrap_or(Ok(Vec::new()))?
            .into_iter()
            .map(|(is_space, area, height, containment)| {
                SpaceOverlap::try_new(oid("other"), is_space, area, height, containment)
            })
            .collect()
    }
    fn measure_cap_coverage(
        &self,
        _space: &ObjectId,
        _cap: Cap,
    ) -> Result<CapCoverage, SpaceError> {
        let (whole, covered) = self.cap.unwrap_or(Ok((10.0, 10.0)))?;
        CapCoverage::try_new(whole, covered, Vec::new())
    }
    fn measure_storey_residuals(&self) -> Result<Vec<StoreyResidual>, SpaceError> {
        self.residuals
            .clone()
            .unwrap_or(Ok(Vec::new()))?
            .into_iter()
            .map(|(storey, area)| StoreyResidual::try_new(storey, area, Vec::new()))
            .collect()
    }
    fn measure_support_counts(&self) -> Result<SupportCounts, SpaceError> {
        let (slabs, roofs) = self.support.unwrap_or(Ok((2, 1)))?;
        Ok(SupportCounts::new(slabs, roofs, vec![oid("bldg")]))
    }
    fn evidence(&self) -> Evidence {
        evidence()
    }
}

fn evaluate(stub: Stub, rule: &CompiledRule) -> axioval_engine::CapabilityEvaluation {
    let project = Project::new(vec![Object::new(oid("space-1"), "space")]).unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(SpaceServiceHandle::new(Arc::new(stub)))
        .unwrap();
    SpaceValidation.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        rule,
    )
}

#[test]
fn a_clean_space_produces_nothing() {
    let outcome = evaluate(Stub::default(), &rule());
    assert!(outcome.findings().is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

#[test]
fn duplicated_space_body_is_an_error() {
    let outcome = evaluate(
        Stub {
            duplicates: Some(Ok(vec![oid("space-2")])),
            ..Stub::default()
        },
        &rule(),
    );
    assert_eq!(outcome.findings().len(), 1);
    assert_eq!(outcome.findings()[0].severity, Severity::Error);
}

#[test]
fn height_below_requirement_is_reported_but_tolerance_is_respected() {
    let low = evaluate(
        Stub {
            height: Some(Ok(2.0)),
            ..Stub::default()
        },
        &rule(),
    );
    assert_eq!(low.findings().len(), 1);
    assert!(low.findings()[0].message.contains("below required"));

    // 2.4995 is short by less than the 5 mm tolerance: measurement noise,
    // not a low space.
    let within_tolerance = evaluate(
        Stub {
            height: Some(Ok(2.4995)),
            ..Stub::default()
        },
        &rule(),
    );
    assert!(within_tolerance.findings().is_empty());
}

#[test]
fn only_boundary_gaps_at_least_the_declared_length_count() {
    let short = evaluate(
        Stub {
            gaps: Some(Ok(vec![(0.4, vec![oid("w1")])])),
            ..Stub::default()
        },
        &rule(),
    );
    assert!(short.findings().is_empty(), "a short gap is an artefact");

    let long = evaluate(
        Stub {
            gaps: Some(Ok(vec![(2.0, vec![oid("w1")])])),
            ..Stub::default()
        },
        &rule(),
    );
    assert_eq!(long.findings().len(), 1);
}

#[test]
fn containment_and_substantial_intersection_are_errors_but_contact_is_not() {
    let contained = evaluate(
        Stub {
            overlaps: Some(Ok(vec![(false, 0.0, 0.0, Containment::SubjectInsideOther)])),
            ..Stub::default()
        },
        &rule(),
    );
    assert_eq!(contained.findings().len(), 1);

    let intersecting = evaluate(
        Stub {
            overlaps: Some(Ok(vec![(true, 2.0, 1.0, Containment::Partial)])),
            ..Stub::default()
        },
        &rule(),
    );
    assert_eq!(intersecting.findings().len(), 1);

    // Touching faces: real contact, not an intersection.
    let touching = evaluate(
        Stub {
            overlaps: Some(Ok(vec![(false, 1.0, 0.001, Containment::Partial)])),
            ..Stub::default()
        },
        &rule(),
    );
    assert!(touching.findings().is_empty());
}

#[test]
fn cap_coverage_is_graded_and_near_complete_coverage_passes() {
    let enabled = [("check_top_cap", ParameterValue::Boolean { value: true })];
    let complete = evaluate(
        Stub {
            cap: Some(Ok((10.0, 9.9))),
            ..Stub::default()
        },
        &rule_with(&enabled),
    );
    assert!(complete.findings().is_empty());

    let bare = evaluate(
        Stub {
            cap: Some(Ok((10.0, 0.05))),
            ..Stub::default()
        },
        &rule_with(&enabled),
    );
    assert_eq!(bare.findings()[0].severity, Severity::Error);

    let partial = evaluate(
        Stub {
            cap: Some(Ok((10.0, 5.0))),
            ..Stub::default()
        },
        &rule_with(&enabled),
    );
    assert_eq!(partial.findings()[0].severity, Severity::Info);
}

/// A cap check without any element that could form the cap says nothing about
/// the space. It must not report every space as uncovered.
#[test]
fn cap_check_is_skipped_when_the_model_has_no_supporting_elements() {
    let outcome = evaluate(
        Stub {
            support: Some(Ok((0, 0))),
            cap: Some(Ok((10.0, 0.0))),
            ..Stub::default()
        },
        &rule_with(&[("check_top_cap", ParameterValue::Boolean { value: true })]),
    );
    assert!(outcome.findings().is_empty());
}

#[test]
fn storey_residual_above_the_allowance_is_reported_against_the_storey() {
    let outcome = evaluate(
        Stub {
            residuals: Some(Ok(vec![(oid("storey-1"), 5.0)])),
            ..Stub::default()
        },
        &rule_with(&[(
            "check_unallocated_area",
            ParameterValue::Boolean { value: true },
        )]),
    );
    assert_eq!(outcome.findings().len(), 1);
    assert_eq!(outcome.findings()[0].object_id, oid("storey-1"));

    let within = evaluate(
        Stub {
            residuals: Some(Ok(vec![(oid("storey-1"), 0.5)])),
            ..Stub::default()
        },
        &rule_with(&[(
            "check_unallocated_area",
            ParameterValue::Boolean { value: true },
        )]),
    );
    assert!(within.findings().is_empty());
}

/// The reason for splitting the bundled fact struct: one unavailable aspect
/// must cost only that aspect. Height is unavailable, yet the duplicate check
/// still reports.
#[test]
fn one_unavailable_aspect_does_not_sink_the_others() {
    let outcome = evaluate(
        Stub {
            height: Some(Err(SpaceError::Unavailable)),
            duplicates: Some(Ok(vec![oid("space-2")])),
            ..Stub::default()
        },
        &rule(),
    );
    assert_eq!(
        outcome.findings().len(),
        1,
        "the duplicate finding must survive an unavailable height"
    );
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::IncompleteEvidence
    );
}

#[test]
fn missing_service_is_neither_a_pass_nor_a_violation() {
    let project = Project::new(vec![Object::new(oid("space-1"), "space")]).unwrap();
    let services = ServiceRegistry::new();
    let outcome = SpaceValidation.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        &rule(),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::MissingService
    );
}

#[test]
fn invalid_declaration_is_refused() {
    let outcome = evaluate(
        Stub::default(),
        &rule_with(&[(
            "required_height_metres",
            ParameterValue::String {
                value: "tall".into(),
            },
        )]),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::InvalidDeclaration
    );
}
