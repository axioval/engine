//! Slab-contact capability contract tests.
//!
//! ADR 0004: the service measures, the capability decides. These pin the
//! decision half, including the two defects the decomposition removed:
//! provider-side rounding, and a provider-side "ignored" verdict.
#![allow(missing_docs)]

use std::{collections::BTreeMap, sync::Arc};

use axioval_engine::{
    CompiledRule, ContactError, ContactEvidence, ContactRequest, ContactService,
    ContactServiceHandle, NotEvaluatedReason, RuleCapability, RuleContext, ServiceRegistry,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity as RuleSeverity};
use axioval_ir::{Evidence, Object, ObjectId, Project, RuleId, Severity, SourceId};
use axioval_rules::SlabContact;

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}
fn oid(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}
fn object(local: &str) -> Object {
    Object::new(oid(local), "wall")
}

fn rule_with(overrides: &[(&str, ParameterValue)]) -> CompiledRule {
    let mut parameters = BTreeMap::from([
        (
            "minimum_contact_ratio".to_string(),
            ParameterValue::Number { value: 0.5 },
        ),
        (
            "contact_side".to_string(),
            ParameterValue::String {
                value: "above".into(),
            },
        ),
        (
            "maximum_gap_metres".to_string(),
            ParameterValue::Number { value: 0.01 },
        ),
        (
            "maximum_intersection_metres".to_string(),
            ParameterValue::Number { value: 0.01 },
        ),
        (
            "minimum_polygon_area_square_metres".to_string(),
            ParameterValue::Number { value: 0.001 },
        ),
    ]);
    for (key, value) in overrides {
        parameters.insert((*key).to_string(), value.clone());
    }
    CompiledRule {
        id: RuleId::new("slab").unwrap(),
        capability: "axioval:capability.slab-contact".into(),
        severity: RuleSeverity::Warning,
        selector: Selector::EntityType {
            object_type: "wall".into(),
            include_subtypes: true,
        },
        parameters,
    }
}
fn rule() -> CompiledRule {
    rule_with(&[])
}

/// whole area, contact area, nearest distance, touching
struct Stub(Result<(f64, f64, Option<f64>, Vec<ObjectId>), ContactError>);

impl ContactService for Stub {
    fn measure_contact(&self, request: &ContactRequest) -> Result<ContactEvidence, ContactError> {
        let (whole, contact, distance, touching) = self.0.clone()?;
        ContactEvidence::try_new(
            request.clone(),
            whole,
            contact,
            distance,
            touching,
            Evidence::exact(source(), "contact:wall"),
        )
    }
}

fn evaluate(stub: Stub, rule: &CompiledRule) -> axioval_engine::CapabilityEvaluation {
    let project = Project::new(vec![object("wall")]).unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(ContactServiceHandle::new(Arc::new(stub)))
        .unwrap();
    SlabContact.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        rule,
    )
}

#[test]
fn contact_meeting_the_minimum_is_not_a_finding() {
    let outcome = evaluate(Stub(Ok((10.0, 6.0, None, Vec::new()))), &rule());
    assert!(outcome.findings().is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

/// The source rounded the ratio to two decimals before comparing. With a
/// minimum of 0.5, a true ratio of 0.495 rounds to 0.50 and passes. The exact
/// measurement must report it as the shortfall it is.
#[test]
fn ratio_just_below_the_minimum_is_not_rounded_into_a_pass() {
    let outcome = evaluate(Stub(Ok((1000.0, 495.0, None, Vec::new()))), &rule());
    assert_eq!(
        outcome.findings().len(),
        1,
        "0.495 must not round up to satisfy a 0.5 minimum"
    );
}

/// A small contact area is a small number, not a state. The source provider
/// classified it as `IgnoredSmall` and the rule skipped it entirely, so a face
/// resting on almost nothing was silently not a violation.
#[test]
fn tiny_contact_area_is_a_finding_not_an_ignored_state() {
    let outcome = evaluate(Stub(Ok((100.0, 0.5, None, vec![oid("slab")]))), &rule());
    assert_eq!(outcome.findings().len(), 1);
    assert!(outcome.findings()[0].message.contains("below required"));
}

#[test]
fn absent_contact_is_graded_by_distance_to_the_nearest_candidate() {
    let near = evaluate(Stub(Ok((10.0, 0.0, Some(0.05), Vec::new()))), &rule());
    assert_eq!(near.findings()[0].severity, Severity::Info);

    let mid = evaluate(Stub(Ok((10.0, 0.0, Some(0.3), Vec::new()))), &rule());
    assert_eq!(mid.findings()[0].severity, Severity::Warning);

    let far = evaluate(Stub(Ok((10.0, 0.0, Some(0.9), Vec::new()))), &rule());
    assert_eq!(far.findings()[0].severity, Severity::Error);

    // Nothing found to rest on at all is the most serious case.
    let unknown = evaluate(Stub(Ok((10.0, 0.0, None, Vec::new()))), &rule());
    assert_eq!(unknown.findings()[0].severity, Severity::Error);
}

#[test]
fn partial_contact_is_graded_by_shortfall() {
    // 0.48 / 0.5 = 0.96 -> marginal
    let marginal = evaluate(Stub(Ok((100.0, 48.0, None, Vec::new()))), &rule());
    assert_eq!(marginal.findings()[0].severity, Severity::Info);
    // 0.10 / 0.5 = 0.20 -> severe
    let severe = evaluate(Stub(Ok((100.0, 10.0, None, Vec::new()))), &rule());
    assert_eq!(severe.findings()[0].severity, Severity::Error);
}

#[test]
fn touching_objects_are_reported_with_the_finding() {
    let outcome = evaluate(
        Stub(Ok((100.0, 10.0, None, vec![oid("slab-a"), oid("slab-b")]))),
        &rule(),
    );
    assert_eq!(outcome.findings().len(), 1);
    assert_eq!(outcome.findings()[0].evidence.len(), 1);
    assert!(outcome.findings()[0].evidence[0].exact);
}

/// An unorientable body was never measured. Treating it as a clean face would
/// turn missing evidence into a silent pass.
#[test]
fn uncheckable_orientation_is_incomplete_evidence_not_a_pass() {
    let outcome = evaluate(Stub(Err(ContactError::UncheckableOrientation)), &rule());
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::IncompleteEvidence
    );
}

#[test]
fn unavailable_measurement_is_not_a_pass() {
    let outcome = evaluate(Stub(Err(ContactError::Unavailable)), &rule());
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::IncompleteEvidence
    );
}

#[test]
fn missing_service_is_neither_a_pass_nor_a_violation() {
    let project = Project::new(vec![object("wall")]).unwrap();
    let services = ServiceRegistry::new();
    let outcome = SlabContact.evaluate(
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
fn invalid_declarations_are_refused() {
    let bad = [
        (
            "minimum_contact_ratio",
            ParameterValue::Number { value: 0.0 },
        ),
        (
            "minimum_contact_ratio",
            ParameterValue::Number { value: 1.5 },
        ),
        (
            "contact_side",
            ParameterValue::String {
                value: "sideways".into(),
            },
        ),
        ("maximum_gap_metres", ParameterValue::Number { value: -1.0 }),
    ];
    for (key, value) in bad {
        let outcome = evaluate(
            Stub(Ok((10.0, 9.0, None, Vec::new()))),
            &rule_with(&[(key, value)]),
        );
        assert!(outcome.findings().is_empty(), "{key} must not evaluate");
        assert_eq!(
            outcome.not_evaluated_outcomes()[0].reason(),
            &NotEvaluatedReason::InvalidDeclaration
        );
    }
}

/// A finding a reviewer cannot act on gets ignored: "insufficient contact" is
/// only useful alongside *what* the face fails to rest on.
#[test]
fn a_shortfall_names_the_objects_the_face_rests_on() {
    let outcome = evaluate(
        Stub(Ok((
            100.0,
            10.0,
            None,
            // Reversed and duplicated: ordering must not depend on the order
            // an adapter happened to walk the model.
            vec![oid("slab-b"), oid("slab-a"), oid("slab-b")],
        ))),
        &rule(),
    );
    assert_eq!(outcome.findings().len(), 1);
    assert_eq!(
        outcome.findings()[0].related,
        vec![oid("slab-a"), oid("slab-b")],
        "touching slabs must be sorted and deduplicated with the finding"
    );
}

/// The subject is already named by `object_id`; repeating it is noise.
#[test]
fn the_subject_is_not_repeated_among_related_objects() {
    let outcome = evaluate(
        Stub(Ok((100.0, 10.0, None, vec![oid("wall"), oid("slab-a")]))),
        &rule(),
    );
    assert_eq!(outcome.findings()[0].related, vec![oid("slab-a")]);
}

/// No contact means nothing to open, unless a nearest candidate was found.
#[test]
fn a_finding_with_nothing_touching_has_no_related_objects() {
    let outcome = evaluate(Stub(Ok((10.0, 0.0, Some(0.3), Vec::new()))), &rule());
    assert!(outcome.findings()[0].related.is_empty());
}
