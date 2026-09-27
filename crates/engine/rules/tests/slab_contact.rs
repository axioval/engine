//! Slab-contact capability contract tests.
//!
//! ADR 0004: the service measures, the capability decides. These pin the
//! decision half, including the two defects the decomposition removed:
//! provider-side rounding, and a provider-side "ignored" verdict.
#![allow(missing_docs)]

mod common;

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
            include_subtypes: false,
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
    // Without a `counterparts` selector every other project object is a
    // candidate, so the slabs the stub reports touching must exist.
    let project = Project::new(vec![
        object("wall"),
        Object::new(oid("slab"), "slab"),
        Object::new(oid("slab-a"), "slab"),
        Object::new(oid("slab-b"), "slab"),
    ])
    .unwrap();
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

/// The subject is never its own candidate. An adapter claiming the face rests
/// on itself answered a different question, so the evidence is refused.
#[test]
fn contact_with_the_subject_itself_is_invalid_evidence() {
    let outcome = evaluate(
        Stub(Ok((100.0, 10.0, None, vec![oid("wall"), oid("slab-a")]))),
        &rule(),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::InvalidEvidence
    );
}

/// No contact means nothing to open, unless a nearest candidate was found.
#[test]
fn a_finding_with_nothing_touching_has_no_related_objects() {
    let outcome = evaluate(Stub(Ok((10.0, 0.0, Some(0.3), Vec::new()))), &rule());
    assert!(outcome.findings()[0].related.is_empty());
}

// ---------------------------------------------------------------------------
// Scope: counterpart selection and storey skipping, over the in-memory source.
// ---------------------------------------------------------------------------

mod scope {
    use std::sync::{Arc, Mutex};

    use axioval_engine::{
        CapabilityEvaluation, ContactError, ContactEvidence, ContactRequest, ContactService,
        ContactServiceHandle, NotEvaluatedReason,
    };
    use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
    use axioval_ir::{ATTRIBUTE_SET, Evidence, ObjectId, PropertyValue, QuantityDimension};
    use axioval_rules::SlabContact;

    use super::common::{Model, boolean, id, kind, number, rule, selector, source, string};

    const ID: &str = "axioval:capability.slab-contact";

    type Seen = Arc<Mutex<Vec<(ObjectId, Vec<ObjectId>)>>>;

    /// Records every request's subject and candidates, and reports `contact`
    /// square metres of a 10 m2 face with nothing touching.
    struct Recording {
        contact: f64,
        seen: Seen,
    }

    impl ContactService for Recording {
        fn measure_contact(
            &self,
            request: &ContactRequest,
        ) -> Result<ContactEvidence, ContactError> {
            self.seen
                .lock()
                .unwrap()
                .push((request.subject().clone(), request.candidates().to_vec()));
            ContactEvidence::try_new(
                request.clone(),
                10.0,
                self.contact,
                None,
                Vec::new(),
                Evidence::exact(source(), "contact"),
            )
        }
    }

    fn elevation(value: f64) -> PropertyValue {
        PropertyValue::Quantity {
            value,
            dimension: QuantityDimension::Length,
        }
    }

    /// Storeys at 0, 3 and 6 m, one wall on each, and two slabs.
    fn building() -> Model {
        let mut model = Model::default()
            .object("slab-a", "slab")
            .object("slab-b", "slab");
        for (storey, wall, height) in [("s0", "w0", 0.0), ("s1", "w1", 3.0), ("s2", "w2", 6.0)] {
            model = model
                .object(storey, "storey")
                .value(storey, ATTRIBUTE_SET, "Elevation", elevation(height))
                .object(wall, "wall")
                .edge("contains", storey, wall);
        }
        model
    }

    struct Checked {
        outcome: CapabilityEvaluation,
        seen: Vec<(ObjectId, Vec<ObjectId>)>,
    }

    impl Checked {
        fn measured(&self) -> Vec<String> {
            self.seen
                .iter()
                .map(|(subject, _)| subject.local_id.clone())
                .collect()
        }
        fn flagged(&self) -> Vec<String> {
            self.outcome
                .findings()
                .iter()
                .map(|finding| finding.object_id().unwrap().local_id.clone())
                .collect()
        }
    }

    /// Checks walls with `extra` parameters added and `drop` ones removed.
    fn check(
        model: Model,
        contact: f64,
        extra: Vec<(&str, ParameterValue)>,
        drop: &[&str],
    ) -> Checked {
        let mut parameters = vec![
            ("minimum_contact_ratio", number(0.5)),
            ("contact_side", string("above")),
            ("maximum_gap_metres", number(0.01)),
            ("maximum_intersection_metres", number(0.01)),
            ("minimum_polygon_area_square_metres", number(0.001)),
            ("counterparts", selector(kind("slab"))),
            ("storey_selector", selector(kind("storey"))),
            ("relationship", string("contains")),
            ("direction", string("backward")),
        ];
        parameters.extend(extra);
        parameters.retain(|(name, _)| !drop.contains(name));
        let seen = Seen::default();
        let recording = Recording {
            contact,
            seen: seen.clone(),
        };
        let outcome = model.evaluate_with(
            &SlabContact,
            &rule(ID, kind("wall"), parameters),
            |services| {
                services
                    .register(ContactServiceHandle::new(Arc::new(recording)))
                    .unwrap();
            },
        );
        let seen = seen.lock().unwrap().clone();
        Checked { outcome, seen }
    }

    fn skip_top() -> Vec<(&'static str, ParameterValue)> {
        vec![("skip_top_storey", boolean(true))]
    }

    #[test]
    fn the_counterpart_selector_is_resolved_into_the_request() {
        let checked = check(building(), 0.0, Vec::new(), &[]);
        assert_eq!(checked.measured(), ["w0", "w1", "w2"]);
        for (_, candidates) in &checked.seen {
            assert_eq!(candidates, &[id("slab-a"), id("slab-b")]);
        }
        assert_eq!(checked.flagged(), ["w0", "w1", "w2"]);
    }

    /// Without a selector the face may rest on anything but itself.
    #[test]
    fn without_a_selector_every_other_object_is_a_candidate() {
        let model = Model::default().object("w", "wall").object("x", "slab");
        let checked = check(model, 10.0, Vec::new(), &["counterparts"]);
        assert_eq!(checked.seen, [(id("w"), vec![id("x")])]);
    }

    #[test]
    fn the_top_storey_is_skipped() {
        let checked = check(building(), 0.0, skip_top(), &[]);
        assert_eq!(checked.measured(), ["w0", "w1"]);
        assert_eq!(checked.flagged(), ["w0", "w1"]);
        assert!(checked.outcome.not_evaluated_outcomes().is_empty());
    }

    #[test]
    fn the_bottom_storey_is_skipped() {
        let checked = check(
            building(),
            0.0,
            vec![("skip_bottom_storey", boolean(true))],
            &[],
        );
        assert_eq!(checked.measured(), ["w1", "w2"]);
        assert_eq!(checked.flagged(), ["w1", "w2"]);
    }

    #[test]
    fn both_storeys_are_skipped_together() {
        let checked = check(
            building(),
            0.0,
            vec![
                ("skip_top_storey", boolean(true)),
                ("skip_bottom_storey", boolean(true)),
            ],
            &[],
        );
        assert_eq!(checked.measured(), ["w1"]);
        assert_eq!(checked.flagged(), ["w1"]);
    }

    /// Order comes from elevation, not from identity or insertion order.
    #[test]
    fn storey_order_follows_elevation() {
        let model = building()
            .object("basement", "storey")
            .value("basement", ATTRIBUTE_SET, "Elevation", elevation(-3.0))
            .object("wb", "wall")
            .edge("contains", "basement", "wb");
        let checked = check(model, 0.0, vec![("skip_bottom_storey", boolean(true))], &[]);
        assert_eq!(checked.measured(), ["w0", "w1", "w2"]);
    }

    /// Without every elevation, which storey is highest is unknown: nothing
    /// is measured and nothing is passed.
    #[test]
    fn an_unknown_elevation_fails_closed() {
        let model = building()
            .object("s3", "storey")
            .object("w3", "wall")
            .edge("contains", "s3", "w3");
        let checked = check(model, 0.0, skip_top(), &[]);
        assert!(checked.seen.is_empty());
        assert!(checked.outcome.findings().is_empty());
        assert_eq!(checked.outcome.not_evaluated_outcomes().len(), 4);
        for entry in checked.outcome.not_evaluated_outcomes() {
            assert_eq!(entry.reason(), &NotEvaluatedReason::IncompleteEvidence);
        }
    }

    /// A subject reaching no storey, or several, has no known position.
    #[test]
    fn a_subject_on_no_single_storey_is_not_evaluated() {
        let model = building()
            .object("loose", "wall")
            .object("tall", "wall")
            .edge("contains", "s0", "tall")
            .edge("contains", "s1", "tall");
        let checked = check(model, 0.0, skip_top(), &[]);
        assert_eq!(checked.measured(), ["w0", "w1"]);
        let unevaluated: Vec<_> = checked
            .outcome
            .not_evaluated_outcomes()
            .iter()
            .map(|entry| (entry.object_id().cloned(), entry.reason().clone()))
            .collect();
        assert_eq!(
            unevaluated,
            [
                (Some(id("loose")), NotEvaluatedReason::IncompleteEvidence),
                (Some(id("tall")), NotEvaluatedReason::IncompleteEvidence),
            ]
        );
    }

    #[test]
    fn skipping_without_storeys_or_a_traversal_is_an_invalid_declaration() {
        for missing in ["storey_selector", "relationship"] {
            let checked = check(building(), 0.0, skip_top(), &[missing]);
            assert!(checked.seen.is_empty(), "{missing}");
            assert_eq!(checked.outcome.not_evaluated_outcomes().len(), 3);
            assert_eq!(
                checked.outcome.not_evaluated_outcomes()[0].reason(),
                &NotEvaluatedReason::InvalidDeclaration,
                "{missing}"
            );
        }
    }

    /// A shortfall is not a finding while an undecided counterpart could
    /// support the face; a pass holds regardless, since more candidates only
    /// add contact.
    #[test]
    fn an_undecided_counterpart_blocks_a_shortfall_but_not_a_pass() {
        let load_bearing = Selector::AllOf {
            operands: vec![
                kind("slab"),
                Selector::Property {
                    property_set: Some("P".into()),
                    property: "LoadBearing".into(),
                    operator: ComparisonOperator::Equals,
                    value: Some(boolean(true)),
                    case_sensitive: true,
                    trim: false,
                    quantifier: None,
                },
            ],
        };
        let model = || {
            building()
                .value("slab-a", "P", "LoadBearing", PropertyValue::Boolean(true))
                .unreadable("slab-b")
        };
        let extra = || vec![("counterparts", selector(load_bearing.clone()))];

        let short = check(model(), 1.0, extra(), &[]);
        assert_eq!(
            short.seen[0].1,
            [id("slab-a")],
            "only decided candidates are sent"
        );
        assert!(short.outcome.findings().is_empty());
        assert_eq!(short.outcome.not_evaluated_outcomes().len(), 3);
        assert_eq!(
            short.outcome.not_evaluated_outcomes()[0].reason(),
            &NotEvaluatedReason::IncompleteEvidence
        );

        let pass = check(model(), 6.0, extra(), &[]);
        assert!(pass.outcome.findings().is_empty());
        assert!(pass.outcome.not_evaluated_outcomes().is_empty());
    }
}
