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
    /// Each distinct request, in the order first asked: the template and
    /// the implementation it is held to each ask once per derivation.
    fn asked(&self) -> Vec<(EnvelopeDerivation, Vec<String>)> {
        let mut distinct: Vec<EnvelopeMembershipRequest> = Vec::new();
        for request in self.asked.lock().unwrap().iter() {
            if !distinct.contains(request) {
                distinct.push(request.clone());
            }
        }
        distinct
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

/// The template, held to the implementation it replaced on every
/// evaluation (`Parity::contract()`).
const HELD: common::Held = common::Held(
    &ExternalWallValidation,
    &axioval_rules::reference::ExternalWallValidation,
);

fn run(model: Model, stub: &Arc<Stub>, rule: &CompiledRule) -> CapabilityEvaluation {
    model.evaluate_measured(&HELD, rule, |services| {
        services
            .register(EnvelopeMembershipServiceHandle::new(stub.clone()))
            .unwrap();
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
        // The envelope's evidence, beside the values read from it.
        assert!(
            finding
                .evidence
                .iter()
                .any(|evidence| evidence.locator == "envelope:all-spaces"),
            "{:?}",
            finding.evidence
        );
        assert!(finding.evidence.iter().all(|evidence| evidence.exact));
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
    let outcome = model().evaluate_measured(&HELD, &all_spaces(), |_| {});
    assert!(outcome.findings().is_empty());
    assert_eq!(
        unevaluated(&outcome),
        [("-".to_owned(), NotEvaluatedReason::MissingService)]
    );
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].message(),
        "envelope-membership service is not registered"
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

/// Nothing selected leaves nothing to say, whatever the declaration: the
/// capability selected before it read its declaration.
#[test]
fn nothing_selected_refuses_nothing() {
    let stub = Stub::sets(&[], &[]);
    let outcome = run(
        model(),
        &stub,
        &rule(
            ID,
            kind("door"),
            vec![("derivations", strings(&["whole-building"]))],
        ),
    );
    assert!(outcome.findings().is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

/// Every message, word for word, as the capability worded it.
#[test]
fn messages_are_worded_as_the_capability_worded_them() {
    let messages = |outcome: &CapabilityEvaluation| {
        let mut messages: Vec<String> = outcome
            .findings()
            .iter()
            .map(|finding| finding.message.clone())
            .chain(
                outcome
                    .not_evaluated_outcomes()
                    .iter()
                    .map(|outcome| outcome.message().to_owned()),
            )
            .collect();
        messages.sort();
        messages
    };
    let both = walls(gross_area(vec![
        ("derivations", strings(&["all-spaces", "gross-area-groups"])),
        ("bounding_selector", selector(kind("space"))),
    ]));
    let stub = Arc::new(Stub {
        answer: Ok((vec![id("w1")], vec![id("w2")])),
        undeclared: vec![id("w3")],
        groups_derived: Some(vec![id("w1"), id("w3")]),
        asked: Mutex::new(Vec::new()),
    });
    assert_eq!(
        messages(&run(model(), &stub, &both)),
        [
            "declared external but not on the all-spaces envelope",
            "not compared with the all-spaces envelope: the model states neither external nor \
             internal, or its body could not be measured",
            "not compared with the gross-area-groups envelope: the model states neither \
             external nor internal, or its body could not be measured",
            "on the all-spaces envelope but not declared external",
            "on the all-spaces envelope but not on the gross-area-groups envelope",
            "on the gross-area-groups envelope but not on the all-spaces envelope",
            "on the gross-area-groups envelope but not on the all-spaces envelope",
        ]
    );
    for (parameters, message) in [
        (
            vec![("derivations", strings(&["all-spaces", "all-spaces"]))],
            "external-wall-validation: `derivations` lists `all-spaces` twice",
        ),
        (
            vec![("derivations", strings(&["whole-building"]))],
            "external-wall-validation: envelope derivation `whole-building` must be \
             'all-spaces' or 'gross-area-groups'",
        ),
        (
            vec![("derivations", strings(&["gross-area-groups", "all-spaces"]))],
            "external-wall-validation: the gross-area-groups derivation needs \
             `gross_area_group_selector` and `gross_area_group_path`",
        ),
        (
            vec![
                ("derivations", strings(&["all-spaces"])),
                ("gross_area_group_selector", selector(kind("zone"))),
            ],
            "external-wall-validation: declare `gross_area_group_selector` and \
             `gross_area_group_path` together",
        ),
    ] {
        assert_eq!(
            messages(&run(model(), &Stub::sets(&[], &[]), &walls(parameters))),
            [message]
        );
    }
    assert_eq!(
        messages(&run(
            model(),
            &Stub::new(Err(EnvelopeMembershipError::Unavailable)),
            &all_spaces()
        )),
        ["all-spaces envelope: envelope membership is unavailable for the requested derivation"]
    );
    assert_eq!(
        messages(&run(model(), &Stub::sets(&[], &["w1"]), &all_spaces())),
        ["no selected object is declared external: the model declares no envelope"]
    );
}

/// Walls in two sources declared, derived and undeclared at random, under
/// one or both derivations bounded by decided, undecided or empty
/// selections, each held to the implementation the template replaced
/// under `Parity::contract()`.
mod generated {
    use std::sync::{Arc, Mutex};

    use axioval_engine::{
        EnvelopeMembershipError, EnvelopeMembershipServiceHandle, ServiceRegistry,
    };
    use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
    use axioval_ir::{ObjectId, SourceId};
    use proptest::collection::vec;
    use proptest::prelude::*;

    use super::common::{Model, kind, rule, selector, strings};
    use super::{ExternalWallValidation, ID, Stub};

    /// One wall: its source (0 or 1), and whether the model declares it
    /// external, the all-spaces derivation places it on the envelope, its
    /// declaration is unknown, and the gross-area-groups derivation places
    /// it on the envelope.
    type Wall = (u32, bool, bool, bool, bool);

    fn wall() -> impl Strategy<Value = Wall> {
        (
            0u32..2,
            any::<bool>(),
            any::<bool>(),
            proptest::bool::weighted(0.2),
            any::<bool>(),
        )
    }

    fn wall_id(index: usize, document: u32) -> ObjectId {
        ObjectId::new(
            SourceId::new("test", if document == 0 { "model" } else { "other" }).unwrap(),
            format!("w{index}"),
        )
        .unwrap()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn generated_walls_hold_parity(
            walls in vec(wall(), 1..6),
            derivations in 0u32..5,
            bounding in 0u32..4,
            groups in any::<bool>(),
            failure in 0u32..6,
            space_on_envelope in any::<bool>(),
        ) {
            let mut model = Model::default()
                .object("s1", "space")
                .object("s2", "space")
                .object("z", "zone")
                .edge("groups", "z", "s1")
                .text("s1", "Pset", "Bounds", "yes");
            if bounding == 2 {
                model = model.unreadable("s2");
            }
            let (mut declared, mut derived, mut undeclared, mut gross) =
                (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for (index, (document, external, on, unknown, on_gross)) in walls.iter().enumerate() {
                model = model.object_in(
                    if *document == 0 { "model" } else { "other" },
                    &format!("w{index}"),
                    "wall",
                );
                let object = wall_id(index, *document);
                if *external {
                    declared.push(object.clone());
                }
                if *on {
                    derived.push(object.clone());
                }
                if *unknown {
                    undeclared.push(object.clone());
                }
                if *on_gross {
                    gross.push(object);
                }
            }
            if space_on_envelope {
                // A bounding space the derivation places on the envelope:
                // the inside, never compared.
                let space = super::id("s1");
                derived.push(space.clone());
                gross.push(space);
            }
            let stub = Arc::new(Stub {
                answer: match failure {
                    0 => Err(EnvelopeMembershipError::Unavailable),
                    1 => Err(EnvelopeMembershipError::InexactEvidence),
                    _ => Ok((declared, derived)),
                },
                undeclared,
                groups_derived: Some(gross),
                asked: Mutex::new(Vec::new()),
            });
            let listed: &[&str] = match derivations {
                0 => &["all-spaces"],
                1 => &["gross-area-groups"],
                2 => &["all-spaces", "gross-area-groups"],
                3 => &["gross-area-groups", "all-spaces"],
                _ => &["all-spaces", "whole-building"],
            };
            let mut parameters: Vec<(&str, ParameterValue)> = vec![("derivations", strings(listed))];
            match bounding {
                0 => {}
                1 => parameters.push(("bounding_selector", selector(kind("space")))),
                2 => parameters.push((
                    "bounding_selector",
                    selector(Selector::property(
                        Some("Pset".into()),
                        "Bounds",
                        ComparisonOperator::Exists,
                        None,
                    )),
                )),
                _ => parameters.push(("bounding_selector", selector(kind("courtyard")))),
            }
            if groups {
                parameters.push(("gross_area_group_selector", selector(kind("zone"))));
                parameters.push(("gross_area_group_path", strings(&["groups:forward"])));
            }
            let register = |services: &mut ServiceRegistry| {
                services
                    .register(EnvelopeMembershipServiceHandle::new(stub.clone()))
                    .unwrap();
            };
            model.holding_contract(
                &ExternalWallValidation,
                &axioval_rules::reference::ExternalWallValidation,
                &rule(ID, kind("wall"), parameters),
                register,
                &[],
                0.0,
            );
        }
    }
}

/// The envelope's evidence is exact and reviewable by contract: a
/// derivation cited approximate is refused before it reaches a value, and
/// the values read from an exact one are exact.
#[test]
fn an_envelope_cited_approximate_is_refused() {
    let mut approximate = Evidence::exact(source(), "envelope");
    approximate.exact = false;
    assert_eq!(
        EnvelopeMembershipEvidence::try_new(
            EnvelopeMembershipRequest::new(EnvelopeDerivation::AllSpaces, vec![id("s1")]),
            Vec::new(),
            Vec::new(),
            1,
            approximate,
        )
        .err(),
        Some(EnvelopeMembershipError::InexactEvidence)
    );
    let read = |stub: Arc<Stub>, name: &str| {
        let (project, mut services) = model().services();
        services
            .register(EnvelopeMembershipServiceHandle::new(stub))
            .unwrap();
        common::measured_cited(&services, &project, &id("w1"), name)
    };
    for name in [
        "on_envelope;derivation=all-spaces;bounding=space",
        "declared_external;derivation=all-spaces;bounding=space",
    ] {
        assert_eq!(
            read(Stub::sets(&["w1"], &["w1"]), name),
            Ok(Some(((1.0, 1.0), true))),
            "{name}"
        );
        assert!(
            read(
                Stub::new(Err(EnvelopeMembershipError::InexactEvidence)),
                name
            )
            .is_err(),
            "{name}"
        );
    }
}
