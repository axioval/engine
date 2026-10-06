//! Slab-contact capability contract tests.
//!
//! ADR 0004: the service measures, the capability decides. These pin the
//! decision half, including the two defects the decomposition removed:
//! provider-side rounding, and a provider-side "ignored" verdict.
//!
//! `slab-contact` runs as a template (#282); every fixture runs it and the
//! implementation it replaced (`axioval_rules::reference::SlabContact`) and
//! holds the template to the whole outside contract (`Parity::contract()`).
#![allow(missing_docs)]

mod common;

use std::{collections::BTreeMap, sync::Arc};

use axioval_engine::{
    CompiledRule, ContactError, ContactEvidence, ContactRequest, ContactService,
    ContactServiceHandle, NotEvaluatedReason, ServiceRegistry,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity as RuleSeverity};
use axioval_ir::{Evidence, Object, ObjectId, Project, RuleId, Severity, SourceId};
use axioval_rules::SlabContact;
use axioval_rules::reference::SlabContact as Reference;

fn source() -> SourceId {
    common::source()
}
fn oid(local: &str) -> ObjectId {
    common::id(local)
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
#[derive(Clone)]
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

/// The template's evaluation, held to the reference's whole contract.
#[allow(clippy::needless_pass_by_value)]
fn evaluate(stub: Stub, rule: &CompiledRule) -> axioval_engine::CapabilityEvaluation {
    // Without a `counterparts` selector every other project object is a
    // candidate, so the slabs the stub reports touching must exist.
    common::Model::default()
        .object("wall", "wall")
        .object("slab", "slab")
        .object("slab-a", "slab")
        .object("slab-b", "slab")
        .holding_contract(
            &SlabContact,
            &Reference,
            rule,
            |services| {
                services
                    .register(ContactServiceHandle::new(Arc::new(stub.clone())))
                    .unwrap();
            },
            &[],
            0.0,
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
    // The contact measurement, cited by each value the template read of it
    // (the share, the area and gap grading it), and the count of undecided
    // counterparts, all exact.
    let evidence = &outcome.findings()[0].evidence;
    assert!(
        evidence
            .iter()
            .any(|evidence| evidence.locator.contains("contact:wall")),
        "{evidence:?}"
    );
    assert!(
        evidence.iter().all(|evidence| evidence.exact),
        "{evidence:?}"
    );
    assert_eq!(
        outcome.findings()[0].related,
        vec![oid("slab-a"), oid("slab-b")]
    );
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
    let outcome = common::Model::default()
        .object("wall", "wall")
        .holding_contract(&SlabContact, &Reference, &rule(), |_| {}, &[], 0.0);
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
    use axioval_rules::parity::{Observations, Parity};
    use axioval_rules::reference::SlabContact as Reference;

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

    /// The rule's parameters, `extra` added and `drop` removed.
    fn declared<'a>(
        extra: Vec<(&'a str, ParameterValue)>,
        drop: &[&str],
    ) -> Vec<(&'a str, ParameterValue)> {
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
        parameters
    }

    /// Checks walls with `extra` parameters added and `drop` ones removed.
    fn check(
        model: Model,
        contact: f64,
        extra: Vec<(&str, ParameterValue)>,
        drop: &[&str],
    ) -> Checked {
        let rule = rule(ID, kind("wall"), declared(extra, drop));
        let recorded = |seen: &Seen| {
            let seen = seen.clone();
            move |services: &mut axioval_engine::ServiceRegistry| {
                services
                    .register(ContactServiceHandle::new(Arc::new(Recording {
                        contact,
                        seen: seen.clone(),
                    })))
                    .unwrap();
            }
        };
        let seen = Seen::default();
        let outcome = model
            .clone()
            .evaluate_measured(&SlabContact, &rule, recorded(&seen));
        let replaced_seen = Seen::default();
        let replaced = model.evaluate_with(&Reference, &rule, recorded(&replaced_seen));
        let parity = Parity::contract().compare(
            (ID, &Observations::of_evaluation(&replaced)),
            ("template", &Observations::of_evaluation(&outcome)),
        );
        assert!(parity.holds(), "{}", parity.diff());
        let seen = seen.lock().unwrap().clone();
        // The template measures exactly what the capability measured.
        assert_eq!(seen, replaced_seen.lock().unwrap().clone());
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
            assert_eq!(
                entry.message(),
                "storeys cannot be ordered: test:model/s3 has no length Elevation (absent)"
            );
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
        assert_eq!(
            checked.outcome.not_evaluated_outcomes()[1].message(),
            "test:model/tall reaches 2 storeys through contains, so whether it is on the top \
             or bottom storey is unknown"
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
                    precision: None,
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
        assert_eq!(
            short.outcome.not_evaluated_outcomes()[0].message(),
            "contact ratio 0.1000 is below required 0.5000, but the counterpart selection is \
             undecided for 1 object(s) that could support the face"
        );

        let pass = check(model(), 6.0, extra(), &[]);
        assert!(pass.outcome.findings().is_empty());
        assert!(pass.outcome.not_evaluated_outcomes().is_empty());
    }

    /// The objects found and left open.
    fn verdicts(outcome: &CapabilityEvaluation) -> (Vec<ObjectId>, Vec<ObjectId>) {
        let mut found: Vec<ObjectId> = outcome
            .findings()
            .iter()
            .filter_map(|finding| finding.object_id().cloned())
            .collect();
        let mut open: Vec<ObjectId> = outcome
            .not_evaluated_outcomes()
            .iter()
            .filter_map(|outcome| outcome.object_id().cloned())
            .collect();
        found.sort();
        open.sort();
        (found, open)
    }

    /// The rule forked from the template, an `expression` rule carrying the
    /// counterparts, side, tolerances, storeys and traversal, reaches the
    /// template's verdicts: it passes a face on a storey left out, finds a
    /// shortfall and leaves open what the template leaves open. Its findings
    /// take the rule's severity, never a graded one. One difference is its
    /// own: an `or` decides where either side does, so a face whose storey
    /// cannot be placed passes the fork where its contact suffices anyway,
    /// which the template, placing the storey first as the capability did,
    /// leaves open.
    #[test]
    fn the_forked_rule_reaches_the_templates_verdicts() {
        use axioval_rules::ExpressionRequirement;
        use axioval_rules::templates::{Fork, fork};
        let skipping = |top: bool, bottom: bool| {
            vec![
                ("skip_top_storey", boolean(top)),
                ("skip_bottom_storey", boolean(bottom)),
            ]
        };
        let models: [&dyn Fn() -> Model; 2] = [&building, &|| {
            building().object("loose", "wall").value(
                "slab-a",
                "P",
                "LoadBearing",
                PropertyValue::Boolean(true),
            )
        }];
        for model in models {
            for contact in [0.0, 1.0, 6.0] {
                for extra in [
                    Vec::new(),
                    skipping(true, false),
                    skipping(false, true),
                    skipping(true, true),
                ] {
                    let bound = rule(ID, kind("wall"), declared(extra, &[]));
                    let forked = fork(&SlabContact, &bound).unwrap();
                    let mut expression_rule = bound.clone();
                    expression_rule.capability = Fork::CAPABILITY.into();
                    expression_rule.parameters = forked.parameters();
                    let register = |services: &mut axioval_engine::ServiceRegistry| {
                        services
                            .register(ContactServiceHandle::new(Arc::new(Recording {
                                contact,
                                seen: Seen::default(),
                            })))
                            .unwrap();
                    };
                    let template = model().evaluate_measured(&SlabContact, &bound, register);
                    let forked = model().evaluate_measured(
                        &ExpressionRequirement,
                        &expression_rule,
                        register,
                    );
                    let ((found, open), (forked_found, forked_open)) =
                        (verdicts(&template), verdicts(&forked));
                    let context = format!("{contact} {:?}", bound.parameters);
                    assert_eq!(found, forked_found, "{context}");
                    assert!(
                        forked_open.iter().all(|object| open.contains(object)),
                        "{context}"
                    );
                    assert!(
                        open.iter()
                            .filter(|object| !forked_open.contains(object))
                            .all(|object| object.local_id == "loose" && contact > 5.0),
                        "{context}"
                    );
                }
            }
        }
    }
}

/// A contact service answering with evidence cited approximate is refused:
/// no contact value is ever measured from an estimate.
#[test]
fn a_contact_cited_approximate_is_refused() {
    struct Approximate;
    impl ContactService for Approximate {
        fn measure_contact(
            &self,
            request: &ContactRequest,
        ) -> Result<ContactEvidence, ContactError> {
            let mut evidence = Evidence::exact(source(), "contact:estimate");
            evidence.exact = false;
            ContactEvidence::try_new(request.clone(), 10.0, 5.0, None, Vec::new(), evidence)
        }
    }
    let project = Project::new(vec![object("wall"), Object::new(oid("slab"), "slab")]).unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(ContactServiceHandle::new(Arc::new(Approximate)))
        .unwrap();
    for name in ["contact_share", "contact_area", "contact_gap"] {
        let measured = common::measured_cited(
            &services,
            &project,
            &oid("wall"),
            &format!("{name};with=slab;side=above"),
        );
        assert!(
            measured
                .as_ref()
                .is_err_and(|why| why.contains("contact evidence must be exact")),
            "{name}: {measured:?}"
        );
    }
}

/// The measured contact share against the minimum ratio reaches the
/// capability's verdict.
#[test]
fn the_measured_contact_share_reaches_the_verdict() {
    let project = Project::new(vec![object("wall"), Object::new(oid("slab"), "slab")]).unwrap();
    for (whole, contact) in [(10.0, 5.0), (10.0, 4.99), (10.0, 0.0), (10.0, 10.0)] {
        let stub = || Stub(Ok((whole, contact, Some(0.2), vec![oid("slab")])));
        let found = !evaluate(stub(), &rule()).findings().is_empty();
        let mut services = ServiceRegistry::new();
        services
            .register(ContactServiceHandle::new(Arc::new(stub())))
            .unwrap();
        let share = common::measured(
            &services,
            &project,
            &oid("wall"),
            "contact_share;with=slab;side=above;gap=0.01;intersection=0.01;polygon=0.001",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            common::at_least(share, 0.5),
            Some(!found),
            "{contact} of {whole}"
        );
    }
}

/// `slab-contact` as one expression rule per severity band over
/// `contact_share`, `contact_area` and `contact_gap`, with `levels_above`
/// and `levels_below` leaving out the top or bottom storey: merged, they
/// reach the capability's verdicts and its graded severities.
#[allow(clippy::needless_pass_by_value)]
mod as_expressions {
    use super::*;
    use axioval_ir::ATTRIBUTE_SET;
    use axioval_ir::{PropertyValue, QuantityDimension};
    use axioval_rules::ExpressionRequirement;
    use common::expressions::{
        above, and, assert_parity, at_least, at_most, below, compare, defined, differences, divide,
        graded, m, m2, measured, merged, not, or, plain,
    };
    use serde_json::Value;

    const ID: &str = "axioval:capability.slab-contact";
    const CONTACT: &str = "with=slab;side=above;gap=0.01;intersection=0.01;polygon=0.001";

    fn value(name: &str) -> Value {
        measured(&format!("{name};{CONTACT}"))
    }

    /// The bands a shortfall falls in, by its severity: an absent contact by
    /// the nearest candidate's distance, a partial one by its share of the
    /// minimum.
    fn band(severity: Severity, minimum: f64) -> Value {
        let gap = || value("contact_gap");
        let relative = || divide(value("contact_share"), plain(minimum));
        let absent = || compare("equals", value("contact_area"), m2(0.0));
        let (absent_band, partial_band) = match severity {
            Severity::Info => (
                and(vec![defined(&gap()), below(gap(), m(0.1))]),
                above(relative(), plain(0.9)),
            ),
            Severity::Error => (
                or(vec![not(defined(&gap())), above(gap(), m(0.5))]),
                below(relative(), plain(0.3)),
            ),
            Severity::Warning => (
                and(vec![
                    defined(&gap()),
                    at_least(gap(), m(0.1)),
                    at_most(gap(), m(0.5)),
                ]),
                and(vec![
                    at_least(relative(), plain(0.3)),
                    at_most(relative(), plain(0.9)),
                ]),
            ),
        };
        and(vec![
            below(value("contact_share"), plain(minimum)),
            or(vec![
                and(vec![absent(), absent_band]),
                and(vec![not(absent()), partial_band]),
            ]),
        ])
    }

    fn rule_severity(severity: &Severity) -> RuleSeverity {
        match severity {
            Severity::Error => RuleSeverity::Error,
            Severity::Warning => RuleSeverity::Warning,
            Severity::Info => RuleSeverity::Info,
        }
    }

    /// One rule per band, each found where the shortfall falls in its band,
    /// passed where the face meets the minimum or `skipped` holds.
    fn rewrite(
        model: &dyn Fn() -> common::Model,
        service: &dyn Fn() -> Arc<dyn ContactService>,
        minimum: f64,
        skipped: Option<(Value, Value)>,
    ) -> axioval_engine::CapabilityEvaluation {
        let evaluations = [Severity::Info, Severity::Warning, Severity::Error]
            .iter()
            .map(|severity| {
                let found = band(severity.clone(), minimum);
                let requirement = match &skipped {
                    None => not(found),
                    Some((placed, skipped)) => {
                        and(vec![placed.clone(), or(vec![skipped.clone(), not(found)])])
                    }
                };
                model().evaluate_measured(
                    &ExpressionRequirement,
                    &graded(kind_of("wall"), &requirement, rule_severity(severity)),
                    |services| {
                        services
                            .register(ContactServiceHandle::new(service()))
                            .unwrap();
                    },
                )
            })
            .collect();
        merged(evaluations)
    }

    fn kind_of(kind: &str) -> Selector {
        Selector::EntityType {
            object_type: kind.into(),
            include_subtypes: false,
        }
    }

    fn slabs() -> common::Model {
        common::Model::default()
            .object("wall", "wall")
            .object("slab", "slab")
            .object("slab-a", "slab")
            .object("slab-b", "slab")
    }

    /// whole area, contact area, nearest distance, touching slabs
    type Answer = Result<(f64, f64, Option<f64>, Vec<&'static str>), ContactError>;

    fn stub(answer: &Answer) -> Arc<dyn ContactService> {
        Arc::new(Stub(answer.clone().map(
            |(whole, contact, gap, touching)| {
                (
                    whole,
                    contact,
                    gap,
                    touching.into_iter().map(common::id).collect(),
                )
            },
        )))
    }

    #[test]
    fn shares_and_their_graded_shortfalls_reach_the_verdicts() {
        let answers: [Answer; 13] = [
            Ok((10.0, 6.0, None, vec![])),
            Ok((1000.0, 495.0, None, vec![])),
            Ok((100.0, 0.5, None, vec!["slab"])),
            Ok((10.0, 0.0, Some(0.05), vec![])),
            Ok((10.0, 0.0, Some(0.3), vec![])),
            Ok((10.0, 0.0, Some(0.9), vec![])),
            Ok((10.0, 0.0, None, vec![])),
            Ok((100.0, 48.0, None, vec![])),
            Ok((100.0, 10.0, None, vec!["slab-a", "slab-b"])),
            Ok((100.0, 10.0, None, vec!["wall", "slab-a"])),
            Err(ContactError::UncheckableOrientation),
            Err(ContactError::Unavailable),
            Err(ContactError::InvalidAreas),
        ];
        for answer in answers {
            for minimum in [0.5, 0.05, 0.9] {
                let declared = rule_with(&[(
                    "minimum_contact_ratio",
                    ParameterValue::Number { value: minimum },
                )]);
                let register = |services: &mut ServiceRegistry| {
                    services
                        .register(ContactServiceHandle::new(stub(&answer)))
                        .unwrap();
                };
                let capability = slabs().evaluate_with(&Reference, &declared, register);
                slabs().holding_contract(&SlabContact, &Reference, &declared, register, &[], 0.0);
                let rewritten = rewrite(&slabs, &|| stub(&answer), minimum, None);
                assert_parity(ID, &capability, &rewritten);
            }
        }
        // Without the service, nothing is judged either way.
        let capability = slabs().evaluate(&Reference, &rule());
        let rewritten = merged(vec![slabs().evaluate_measured(
            &ExpressionRequirement,
            &graded(
                kind_of("wall"),
                &not(band(Severity::Error, 0.5)),
                RuleSeverity::Error,
            ),
            |_| {},
        )]);
        assert_parity(ID, &capability, &rewritten);
    }

    /// Storeys at 0, 3 and 6 m, a wall on each, and two slabs.
    fn building() -> common::Model {
        let mut model = common::Model::default()
            .object("slab-a", "slab")
            .object("slab-b", "slab");
        for (storey, wall, height) in [("s0", "w0", 0.0), ("s1", "w1", 3.0), ("s2", "w2", 6.0)] {
            model = model
                .object(storey, "storey")
                .value(
                    storey,
                    ATTRIBUTE_SET,
                    "Elevation",
                    PropertyValue::Quantity {
                        value: height,
                        dimension: QuantityDimension::Length,
                    },
                )
                .object(wall, "wall")
                .edge("contains", storey, wall);
        }
        model
    }

    fn skipping(top: bool, bottom: bool) -> Vec<(&'static str, ParameterValue)> {
        vec![
            ("skip_top_storey", ParameterValue::Boolean { value: top }),
            (
                "skip_bottom_storey",
                ParameterValue::Boolean { value: bottom },
            ),
            (
                "counterparts",
                ParameterValue::Selector {
                    value: Box::new(kind_of("slab")),
                },
            ),
            (
                "storey_selector",
                ParameterValue::Selector {
                    value: Box::new(kind_of("storey")),
                },
            ),
            (
                "relationship",
                ParameterValue::String {
                    value: "contains".into(),
                },
            ),
            (
                "direction",
                ParameterValue::String {
                    value: "backward".into(),
                },
            ),
        ]
    }

    /// Whether the face's storey is placed among the storeys, and whether
    /// it is the top storey (`top`) or the bottom one, so left out. The
    /// capability measures nothing while a storey is unplaced, so neither
    /// does a passing face pass then.
    fn skipped(top: bool, bottom: bool) -> Option<(Value, Value)> {
        let count = |name: &str| measured(&format!("{name};levels=storey;path=contains:backward"));
        let zero = || common::expressions::integer(0);
        let mut placed = Vec::new();
        let mut ends = Vec::new();
        for (end, name) in [(top, "levels_above"), (bottom, "levels_below")] {
            if end {
                placed.push(at_least(count(name), zero()));
                ends.push(compare("equals", count(name), zero()));
            }
        }
        (!ends.is_empty()).then(|| (and(placed), or(ends)))
    }

    fn untouched(contact: f64) -> Arc<dyn ContactService> {
        Arc::new(Stub(Ok((10.0, contact, None, Vec::new()))))
    }

    #[test]
    fn storeys_left_out_by_elevation_reach_the_verdicts() {
        let basement = || {
            building()
                .object("basement", "storey")
                .value(
                    "basement",
                    ATTRIBUTE_SET,
                    "Elevation",
                    PropertyValue::Quantity {
                        value: -3.0,
                        dimension: QuantityDimension::Length,
                    },
                )
                .object("wb", "wall")
                .edge("contains", "basement", "wb")
        };
        let unordered = || {
            building()
                .object("s3", "storey")
                .object("w3", "wall")
                .edge("contains", "s3", "w3")
        };
        let loose = || {
            building()
                .object("loose", "wall")
                .object("tall", "wall")
                .edge("contains", "s0", "tall")
                .edge("contains", "s1", "tall")
        };
        let models: [&dyn Fn() -> common::Model; 4] = [&building, &basement, &unordered, &loose];
        for model in models {
            for (top, bottom) in [(true, false), (false, true), (true, true)] {
                for contact in [0.0, 6.0] {
                    let mut capability_rule = rule();
                    capability_rule.selector = kind_of("wall");
                    for (name, value) in skipping(top, bottom) {
                        capability_rule.parameters.insert(name.into(), value);
                    }
                    let register = |s: &mut ServiceRegistry| {
                        s.register(ContactServiceHandle::new(untouched(contact)))
                            .unwrap();
                    };
                    let capability = model().evaluate_with(&Reference, &capability_rule, register);
                    model().holding_contract(
                        &SlabContact,
                        &Reference,
                        &capability_rule,
                        register,
                        &[],
                        0.0,
                    );
                    let rewritten =
                        rewrite(model, &|| untouched(contact), 0.5, skipped(top, bottom));
                    assert_parity(ID, &capability, &rewritten);
                }
            }
        }
    }

    /// A counterpart named by a property, not a kind, cannot be a measured
    /// value's candidate: the rewrite sends every slab, so an undecided one
    /// counts as a candidate where the capability leaves the face open.
    #[test]
    fn an_undecided_counterpart_is_a_candidate_of_the_rewrite() {
        use axioval_ir::contract::ComparisonOperator;
        let load_bearing = Selector::AllOf {
            operands: vec![
                kind_of("slab"),
                Selector::property(
                    Some("P".into()),
                    "LoadBearing",
                    ComparisonOperator::Equals,
                    Some(ParameterValue::Boolean { value: true }),
                ),
            ],
        };
        let model = || {
            building()
                .value("slab-a", "P", "LoadBearing", PropertyValue::Boolean(true))
                .unreadable("slab-b")
        };
        let mut capability_rule = rule();
        capability_rule.selector = kind_of("wall");
        capability_rule.parameters.insert(
            "counterparts".into(),
            ParameterValue::Selector {
                value: Box::new(load_bearing),
            },
        );
        let capability = model().evaluate_with(&Reference, &capability_rule, |s| {
            s.register(ContactServiceHandle::new(untouched(1.0)))
                .unwrap();
        });
        let rewritten = rewrite(&model, &|| untouched(1.0), 0.5, None);
        assert_eq!(
            differences(ID, &capability, &rewritten),
            ["w0", "w1", "w2"].map(|wall| format!(
                "test:model/{wall}: capability not evaluated (IncompleteEvidence), \
                 expression finding (Error, exact evidence)"
            ))
        );
    }
}

/// Generated walls on storeys of random elevations, resting on slabs a
/// selector may or may not pick, with random contact, gaps, minimums and
/// storeys left out: the template holds the replaced implementation's whole
/// contract.
mod generated {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use axioval_engine::{
        ContactError, ContactEvidence, ContactRequest, ContactService, ContactServiceHandle,
        ServiceRegistry,
    };
    use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
    use axioval_ir::{ATTRIBUTE_SET, Evidence, ObjectId, PropertyValue, QuantityDimension};
    use axioval_rules::SlabContact;
    use axioval_rules::reference::SlabContact as Reference;
    use proptest::collection::vec;
    use proptest::prelude::*;

    use super::common::{Model, boolean, id, kind, number, rule, selector, source, string};

    const ID: &str = "axioval:capability.slab-contact";

    /// One wall: its storey (0 to 2, 3 none), the quarters of its 16 m²
    /// face in contact (0 to 64), the gap to the nearest candidate in
    /// tenths of a metre (none above 9), how many candidates it touches,
    /// and the service's answer (0 to 3 measured, 4 unavailable, 5 not
    /// orientable).
    type Wall = (u32, u32, u32, u32, u32);

    fn wall() -> impl Strategy<Value = Wall> {
        (0u32..4, 0u32..65, 0u32..12, 0u32..3, 0u32..6)
    }

    /// Answers each wall's request from its row.
    #[derive(Clone)]
    struct Table(BTreeMap<ObjectId, Wall>);

    impl ContactService for Table {
        fn measure_contact(
            &self,
            request: &ContactRequest,
        ) -> Result<ContactEvidence, ContactError> {
            let Some((_, quarters, gap, touch, answer)) = self.0.get(request.subject()) else {
                return Err(ContactError::Unavailable);
            };
            match answer {
                4 => return Err(ContactError::Unavailable),
                5 => return Err(ContactError::UncheckableOrientation),
                _ => {}
            }
            let contact = f64::from(*quarters) / 4.0;
            let touching = if contact > 0.0 {
                request
                    .candidates()
                    .iter()
                    .take(*touch as usize)
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            };
            ContactEvidence::try_new(
                request.clone(),
                16.0,
                contact,
                (*gap < 10).then(|| f64::from(*gap) / 10.0),
                touching,
                Evidence::exact(source(), format!("contact:{}", request.subject())),
            )
        }
    }

    /// The model: three storeys (`elevations` in metres, none where
    /// absent), the walls on them, and slabs each picked surely (0),
    /// undecidably (1) or not (2) by the load-bearing selector.
    fn fixture(elevations: &[Option<i32>], walls: &[Wall], slabs: &[u32]) -> (Model, Table) {
        let mut model = Model::default();
        for (index, elevation) in elevations.iter().enumerate() {
            let storey = format!("s{index}");
            model = model.object(&storey, "storey");
            if let Some(elevation) = elevation {
                model = model.value(
                    &storey,
                    ATTRIBUTE_SET,
                    "Elevation",
                    PropertyValue::Quantity {
                        value: f64::from(*elevation),
                        dimension: QuantityDimension::Length,
                    },
                );
            }
        }
        let mut table = BTreeMap::new();
        for (index, row) in walls.iter().enumerate() {
            let local = format!("w{index}");
            model = model.object(&local, "wall");
            if row.0 < 3 {
                model = model.edge("contains", &format!("s{}", row.0), &local);
            }
            table.insert(id(&local), *row);
        }
        for (index, picked) in slabs.iter().enumerate() {
            let local = format!("slab{index}");
            model = model.object(&local, "slab");
            model = match picked {
                0 => model.value(&local, "P", "LoadBearing", PropertyValue::Boolean(true)),
                1 => model.unreadable(&local),
                _ => model.value(&local, "P", "LoadBearing", PropertyValue::Boolean(false)),
            };
        }
        (model, Table(table))
    }

    fn load_bearing() -> Selector {
        Selector::AllOf {
            operands: vec![
                kind("slab"),
                Selector::property(
                    Some("P".into()),
                    "LoadBearing",
                    ComparisonOperator::Equals,
                    Some(boolean(true)),
                ),
            ],
        }
    }

    fn parameters(
        minimum: u32,
        above: bool,
        counterparts: bool,
        (top, bottom): (bool, bool),
    ) -> Vec<(&'static str, ParameterValue)> {
        let mut parameters = vec![
            ("minimum_contact_ratio", number(f64::from(minimum) / 16.0)),
            (
                "contact_side",
                string(if above { "above" } else { "below" }),
            ),
            ("maximum_gap_metres", number(0.01)),
            ("maximum_intersection_metres", number(0.01)),
            ("minimum_polygon_area_square_metres", number(0.001)),
        ];
        if counterparts {
            parameters.push(("counterparts", selector(load_bearing())));
        }
        if top || bottom {
            parameters.extend([
                ("skip_top_storey", boolean(top)),
                ("skip_bottom_storey", boolean(bottom)),
                ("storey_selector", selector(kind("storey"))),
                ("relationship", string("contains")),
                ("direction", string("backward")),
            ]);
        }
        parameters
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn generated_faces_hold_parity(
            elevations in vec(proptest::option::weighted(0.9, -6i32..12), 3),
            walls in vec(wall(), 1..6),
            slabs in vec(0u32..3, 0..4),
            minimum in 1u32..17,
            above in any::<bool>(),
            counterparts in any::<bool>(),
            skipping in (any::<bool>(), any::<bool>()),
        ) {
            let (model, table) = fixture(&elevations, &walls, &slabs);
            let register = |services: &mut ServiceRegistry| {
                services
                    .register(ContactServiceHandle::new(Arc::new(table.clone())))
                    .unwrap();
            };
            model.holding_contract(
                &SlabContact,
                &Reference,
                &rule(ID, kind("wall"), parameters(minimum, above, counterparts, skipping)),
                register,
                &[],
                0.0,
            );
        }
    }
}

/// Every message, word for word, as the capability worded it.
#[test]
fn messages_are_worded_as_the_capability_worded_them() {
    let message = |stub: Stub, rule: &CompiledRule| {
        let outcome = evaluate(stub, rule);
        outcome
            .findings()
            .iter()
            .map(|finding| finding.message.clone())
            .chain(
                outcome
                    .not_evaluated_outcomes()
                    .iter()
                    .map(|outcome| outcome.message().to_owned()),
            )
            .collect::<Vec<_>>()
    };
    assert_eq!(
        message(Stub(Ok((10.0, 0.0, Some(0.3), Vec::new()))), &rule()),
        ["no contact"]
    );
    assert_eq!(
        message(Stub(Ok((100.0, 10.0, None, Vec::new()))), &rule()),
        ["contact ratio 0.1000 below required 0.5000"]
    );
    assert_eq!(
        message(Stub(Err(ContactError::UncheckableOrientation)), &rule()),
        ["object body has no checkable orientation"]
    );
    assert_eq!(
        message(
            Stub(Ok((10.0, 9.0, None, Vec::new()))),
            &rule_with(&[(
                "minimum_contact_ratio",
                ParameterValue::Number { value: 1.5 }
            )])
        ),
        ["slab-contact declaration is invalid: minimum_contact_ratio must lie in (0, 1]"]
    );
    assert_eq!(
        message(
            Stub(Ok((10.0, 9.0, None, Vec::new()))),
            &rule_with(&[(
                "contact_side",
                ParameterValue::String {
                    value: "sideways".into()
                }
            )])
        ),
        ["slab-contact declaration is invalid: contact_side `sideways` is unsupported"]
    );
    assert_eq!(
        message(
            Stub(Ok((10.0, 9.0, None, Vec::new()))),
            &rule_with(&[("maximum_gap_metres", ParameterValue::Number { value: -1.0 })])
        ),
        [
            "slab-contact declaration is invalid: contact areas must be finite, non-negative \
             and contained"
        ]
    );
    assert_eq!(
        message(
            Stub(Ok((10.0, 9.0, None, Vec::new()))),
            &rule_with(&[("skip_top_storey", ParameterValue::Boolean { value: true })])
        ),
        [
            "slab-contact declaration is invalid: skipping a storey needs `storey_selector` \
             to say what a storey is"
        ]
    );
}
