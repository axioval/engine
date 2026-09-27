//! External-wall-validation capability contract tests.
//!
//! ADR 0004: the service measures envelope membership, the capability decides
//! whether the declaration agrees with it. The rule, not the host, names the
//! objects each derivation is bounded by.
#![allow(missing_docs)]

mod common;

use std::sync::{Arc, Mutex};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, EnvelopeDerivation, EnvelopeMembershipError,
    EnvelopeMembershipEvidence, EnvelopeMembershipRequest, EnvelopeMembershipService,
    EnvelopeMembershipServiceHandle, NotEvaluatedReason,
};
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_ir::{Evidence, ObjectId};
use axioval_rules::ExternalWallValidation;
use common::{Model, findings, id, kind, rule, selector, source, strings, unevaluated};

const ID: &str = "axioval:capability.external-wall-validation";

/// Answers every request with the same `(declared, derived)` sets, or an
/// error, and records what it was asked.
struct Stub {
    answer: Result<(Vec<ObjectId>, Vec<ObjectId>), EnvelopeMembershipError>,
    undeclared: Vec<ObjectId>,
    /// The derived set of the gross-area-groups derivation, when it differs.
    groups_derived: Option<Vec<ObjectId>>,
    asked: Mutex<Vec<EnvelopeMembershipRequest>>,
}

impl Stub {
    fn new(answer: Result<(Vec<ObjectId>, Vec<ObjectId>), EnvelopeMembershipError>) -> Arc<Self> {
        Arc::new(Self {
            answer,
            undeclared: Vec::new(),
            groups_derived: None,
            asked: Mutex::new(Vec::new()),
        })
    }
    fn sets(declared: &[&str], derived: &[&str]) -> Arc<Self> {
        Self::new(Ok((
            declared.iter().map(|o| id(o)).collect(),
            derived.iter().map(|o| id(o)).collect(),
        )))
    }
    fn asked(&self) -> Vec<(EnvelopeDerivation, Vec<String>)> {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .map(|request| {
                (
                    request.derivation(),
                    request
                        .bounding()
                        .iter()
                        .map(|o| o.local_id.clone())
                        .collect(),
                )
            })
            .collect()
    }
}

impl EnvelopeMembershipService for Stub {
    fn measure_envelope_membership(
        &self,
        request: &EnvelopeMembershipRequest,
    ) -> Result<EnvelopeMembershipEvidence, EnvelopeMembershipError> {
        self.asked.lock().unwrap().push(request.clone());
        let (declared, mut derived) = self.answer.clone()?;
        if request.derivation() == EnvelopeDerivation::GrossAreaGroups
            && let Some(groups) = &self.groups_derived
        {
            derived.clone_from(groups);
        }
        Ok(EnvelopeMembershipEvidence::try_new(
            request.clone(),
            declared,
            derived,
            3,
            Evidence::exact(
                source(),
                format!("envelope:{}", request.derivation().as_str()),
            ),
        )?
        .with_undeclared(self.undeclared.clone()))
    }
}

/// Walls w1..w3, spaces s1 and s2, zone z grouping s1, and a slab.
fn model() -> Model {
    Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .object("w3", "wall")
        .object("s1", "space")
        .object("s2", "space")
        .object("z", "zone")
        .object("slab", "slab")
        .edge("groups", "z", "s1")
}

fn walls(parameters: Vec<(&str, ParameterValue)>) -> CompiledRule {
    rule(ID, kind("wall"), parameters)
}

/// The `all-spaces` derivation around every space.
fn all_spaces() -> CompiledRule {
    walls(vec![
        ("derivations", strings(&["all-spaces"])),
        ("bounding_selector", selector(kind("space"))),
    ])
}

fn gross_area(
    mut parameters: Vec<(&'static str, ParameterValue)>,
) -> Vec<(&'static str, ParameterValue)> {
    parameters.push(("gross_area_group_selector", selector(kind("zone"))));
    parameters.push(("gross_area_group_path", strings(&["groups:forward"])));
    parameters
}

fn run(model: Model, stub: &Arc<Stub>, rule: &CompiledRule) -> CapabilityEvaluation {
    let handle = EnvelopeMembershipServiceHandle::new(stub.clone());
    model.evaluate_with(&ExternalWallValidation, rule, |services| {
        services.register(handle).unwrap();
    })
}

#[test]
fn matching_declaration_and_derivation_is_not_a_finding() {
    let outcome = run(model(), &Stub::sets(&["w1"], &["w1"]), &all_spaces());
    assert!(outcome.findings().is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

/// The rule's bounding selection travels in the request, exactly.
#[test]
fn the_bounding_selector_is_carried_in_the_request() {
    let stub = Stub::sets(&[], &[]);
    run(model(), &stub, &all_spaces());
    assert_eq!(
        stub.asked(),
        [(
            EnvelopeDerivation::AllSpaces,
            vec!["s1".to_owned(), "s2".to_owned()]
        )]
    );
}

/// Gross-area groups are resolved along the declared path; their members,
/// not the groups, bound the derivation.
#[test]
fn gross_area_groups_are_bounded_by_their_members() {
    let stub = Stub::sets(&[], &[]);
    run(
        model(),
        &stub,
        &walls(gross_area(vec![(
            "derivations",
            strings(&["gross-area-groups"]),
        )])),
    );
    assert_eq!(
        stub.asked(),
        [(EnvelopeDerivation::GrossAreaGroups, vec!["s1".to_owned()])]
    );
}

/// Both derivations run in one rule, each with its own bounding set, and
/// every finding names the derivation it came from.
#[test]
fn both_derivations_are_measured_and_reported_separately() {
    let stub = Stub::sets(&["w1"], &["w2"]);
    let outcome = run(
        model(),
        &stub,
        &walls(gross_area(vec![
            ("derivations", strings(&["all-spaces", "gross-area-groups"])),
            ("bounding_selector", selector(kind("space"))),
        ])),
    );
    assert_eq!(
        stub.asked(),
        [
            (
                EnvelopeDerivation::AllSpaces,
                vec!["s1".to_owned(), "s2".to_owned()]
            ),
            (EnvelopeDerivation::GrossAreaGroups, vec!["s1".to_owned()]),
        ]
    );
    let mut reported = findings(&outcome);
    reported.sort();
    assert_eq!(
        reported,
        [
            (
                "w1".to_owned(),
                "declared external but not on the all-spaces envelope".to_owned()
            ),
            (
                "w1".to_owned(),
                "declared external but not on the gross-area-groups envelope".to_owned()
            ),
            (
                "w2".to_owned(),
                "on the all-spaces envelope but not declared external".to_owned()
            ),
            (
                "w2".to_owned(),
                "on the gross-area-groups envelope but not declared external".to_owned()
            ),
        ]
    );
}

/// A derivation without its bounding input is refused before anything is
/// measured, never derived around nothing or a host default.
#[test]
fn a_derivation_without_its_bounding_input_is_an_invalid_declaration() {
    for parameters in [
        vec![("derivations", strings(&["all-spaces"]))],
        vec![
            ("derivations", strings(&["gross-area-groups"])),
            ("bounding_selector", selector(kind("space"))),
        ],
        vec![
            ("derivations", strings(&["gross-area-groups"])),
            ("gross_area_group_selector", selector(kind("zone"))),
        ],
        vec![("derivations", strings(&[]))],
        vec![("derivations", strings(&["whole-building"]))],
        vec![("derivations", strings(&["all-spaces", "all-spaces"]))],
        vec![(
            "derivations",
            ParameterValue::String {
                value: "all-spaces".into(),
            },
        )],
    ] {
        let stub = Stub::sets(&[], &[]);
        let outcome = run(model(), &stub, &walls(parameters.clone()));
        assert!(outcome.findings().is_empty());
        assert_eq!(
            unevaluated(&outcome),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)],
            "{parameters:?}"
        );
        assert!(stub.asked().is_empty(), "{parameters:?}");
    }
}

/// An object the bounding selector cannot decide might bound the envelope, so
/// that derivation is not evaluated; the other one still is.
#[test]
fn an_undecided_bounding_selection_is_not_evaluated() {
    let marked = Selector::property(
        Some("Pset".into()),
        "Bounds",
        ComparisonOperator::Exists,
        None,
    );
    let stub = Stub::sets(&["w1"], &["w1"]);
    let outcome = run(
        model().text("s1", "Pset", "Bounds", "yes").unreadable("s2"),
        &stub,
        &walls(gross_area(vec![
            ("derivations", strings(&["all-spaces", "gross-area-groups"])),
            ("bounding_selector", selector(marked)),
        ])),
    );
    assert_eq!(
        unevaluated(&outcome),
        [("-".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert!(
        outcome.not_evaluated_outcomes()[0]
            .message()
            .contains("all-spaces")
    );
    assert_eq!(
        stub.asked(),
        [(EnvelopeDerivation::GrossAreaGroups, vec!["s1".to_owned()])]
    );
}

/// Nothing selected, or groups without members, leave no region.
#[test]
fn an_empty_bounding_set_is_not_evaluated() {
    let stub = Stub::sets(&[], &[]);
    let outcome = run(
        model(),
        &stub,
        &walls(vec![
            ("derivations", strings(&["all-spaces"])),
            ("bounding_selector", selector(kind("courtyard"))),
        ]),
    );
    assert_eq!(
        unevaluated(&outcome),
        [("-".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    let outcome = run(
        Model::default()
            .object("w1", "wall")
            .object("z", "zone")
            .edge("groups", "z", "w1")
            .edge("other", "z", "w1"),
        &stub,
        &walls(vec![
            ("derivations", strings(&["gross-area-groups"])),
            ("gross_area_group_selector", selector(kind("zone"))),
            ("gross_area_group_path", strings(&["other:backward"])),
        ]),
    );
    assert_eq!(
        unevaluated(&outcome),
        [("-".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert!(stub.asked().is_empty());
}

/// A refused relationship answer along the group path leaves the derivation
/// not evaluated, never bounded by fewer members.
#[test]
fn a_refused_group_path_is_not_evaluated() {
    let stub = Stub::sets(&[], &[]);
    let outcome = run(
        model(),
        &stub,
        &walls(vec![
            ("derivations", strings(&["gross-area-groups"])),
            ("gross_area_group_selector", selector(kind("zone"))),
            ("gross_area_group_path", strings(&["unknown:forward"])),
        ]),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        unevaluated(&outcome),
        [("-".to_owned(), NotEvaluatedReason::BackendUnavailable)]
    );
    assert!(stub.asked().is_empty());
}

/// Order and duplicates are an adapter artefact, not a discrepancy.
#[test]
fn agreement_is_order_and_duplicate_insensitive() {
    let outcome = run(
        model(),
        &Stub::sets(&["w2", "w1", "w2"], &["w1", "w2"]),
        &all_spaces(),
    );
    assert!(outcome.findings().is_empty());
}

/// Each disagreeing wall is reported against itself, so a reviewer opens the
/// element rather than a whole-model message.
#[test]
fn disagreement_is_reported_per_element_in_both_directions() {
    let outcome = run(
        model(),
        &Stub::sets(&["w1", "w2"], &["w2", "w3"]),
        &all_spaces(),
    );
    assert_eq!(
        findings(&outcome),
        [
            (
                "w1".to_owned(),
                "declared external but not on the all-spaces envelope".to_owned()
            ),
            (
                "w3".to_owned(),
                "on the all-spaces envelope but not declared external".to_owned()
            ),
        ]
    );
}

/// A model declaring nothing external while geometry finds walls is a real
/// discrepancy, not a vacuous pass: one major finding against the source,
/// never one per wall on the envelope.
#[test]
fn nothing_declared_external_is_one_source_finding() {
    let outcome = run(model(), &Stub::sets(&[], &["w1", "w2"]), &all_spaces());
    assert_eq!(outcome.findings().len(), 1, "{:?}", outcome.findings());
    let finding = &outcome.findings()[0];
    assert_eq!(finding.scope, axioval_ir::Scope::Source(source()));
    assert_eq!(finding.severity, axioval_ir::Severity::Error);
    assert!(
        finding
            .message
            .contains("no selected object is declared external")
    );
    assert!(outcome.not_evaluated_outcomes().is_empty());

    // Both derivations measured: still one finding for the source.
    let outcome = run(
        model(),
        &Stub::sets(&[], &["w1"]),
        &walls(gross_area(vec![
            ("derivations", strings(&["all-spaces", "gross-area-groups"])),
            ("bounding_selector", selector(kind("space"))),
        ])),
    );
    assert_eq!(findings(&outcome).len(), 1, "{:?}", findings(&outcome));
}

/// A selected wall stating neither might be the external one, so the
/// source is not evaluated rather than found.
#[test]
fn nothing_declared_external_with_an_undeclared_wall_is_not_evaluated() {
    let stub = Arc::new(Stub {
        answer: Ok((Vec::new(), vec![id("w1")])),
        undeclared: vec![id("w2")],
        groups_derived: None,
        asked: Mutex::new(Vec::new()),
    });
    let outcome = run(model(), &stub, &all_spaces());
    assert_eq!(
        findings(&outcome),
        [(
            "w1".to_owned(),
            "on the all-spaces envelope but not declared external".to_owned()
        )]
    );
    let sources: Vec<_> = outcome
        .not_evaluated_outcomes()
        .iter()
        .filter(|outcome| matches!(outcome.scope(), axioval_ir::Scope::Source(_)))
        .collect();
    assert_eq!(sources.len(), 1);
    assert!(sources[0].message().contains("1 state neither"));
}

/// With both derivations, an object on one envelope and not the other is
/// reported against itself, whatever it declares.
#[test]
fn an_object_on_only_one_derived_envelope_is_a_disagreement() {
    let stub = Arc::new(Stub {
        answer: Ok((vec![id("w1"), id("w2")], vec![id("w1"), id("w2")])),
        undeclared: vec![id("w3")],

        // w2 is on the all-spaces envelope only; w3 (declaring nothing) on
        // the gross-area-groups envelope only.
        groups_derived: Some(vec![id("w1"), id("w3")]),
        asked: Mutex::new(Vec::new()),
    });
    let outcome = run(
        model(),
        &stub,
        &walls(gross_area(vec![
            ("derivations", strings(&["all-spaces", "gross-area-groups"])),
            ("bounding_selector", selector(kind("space"))),
        ])),
    );
    let mut reported = findings(&outcome);
    reported.sort();
    assert_eq!(
        reported,
        [
            (
                "w2".to_owned(),
                "declared external but not on the gross-area-groups envelope".to_owned()
            ),
            (
                "w2".to_owned(),
                "on the all-spaces envelope but not on the gross-area-groups envelope".to_owned()
            ),
            (
                "w3".to_owned(),
                "on the gross-area-groups envelope but not on the all-spaces envelope".to_owned()
            ),
        ]
    );
    // One derivation alone has nothing to disagree with.
    let outcome = run(
        model(),
        &Stub::sets(&["w1"], &["w1"]),
        &walls(gross_area(vec![(
            "derivations",
            strings(&["gross-area-groups"]),
        )])),
    );
    assert!(outcome.findings().is_empty());
}

#[test]
fn every_finding_carries_its_evidence() {
    let outcome = run(model(), &Stub::sets(&["w1"], &["w2"]), &all_spaces());
    assert!(!outcome.findings().is_empty());
    for finding in outcome.findings() {
        assert_eq!(finding.evidence.len(), 1);
        assert!(finding.evidence[0].exact);
    }
}

#[test]
fn unavailable_or_unsupported_measurement_is_not_a_pass() {
    for error in [
        EnvelopeMembershipError::UnsupportedDerivation,
        EnvelopeMembershipError::Unavailable,
    ] {
        let outcome = run(model(), &Stub::new(Err(error)), &all_spaces());
        assert!(outcome.findings().is_empty());
        assert_eq!(
            unevaluated(&outcome),
            [("-".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
        );
    }
}

#[test]
fn missing_service_is_neither_a_pass_nor_a_violation() {
    let outcome = model().evaluate(&ExternalWallValidation, &all_spaces());
    assert!(outcome.findings().is_empty());
    assert_eq!(
        unevaluated(&outcome),
        [("-".to_owned(), NotEvaluatedReason::MissingService)]
    );
}

/// An undeclared wall is not evaluated, never read as internal, and a
/// disagreement outside the selection is not this rule's finding.
#[test]
fn undeclared_walls_are_not_evaluated_and_unselected_objects_are_ignored() {
    let stub = Arc::new(Stub {
        answer: Ok((vec![id("w1")], vec![id("w1"), id("w2"), id("slab")])),
        undeclared: vec![id("w2")],
        groups_derived: None,
        asked: Mutex::new(Vec::new()),
    });
    let outcome = run(model(), &stub, &all_spaces());
    assert!(outcome.findings().is_empty(), "{:?}", outcome.findings());
    assert_eq!(
        unevaluated(&outcome),
        [("w2".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert!(
        outcome.not_evaluated_outcomes()[0]
            .message()
            .contains("all-spaces")
    );
}
