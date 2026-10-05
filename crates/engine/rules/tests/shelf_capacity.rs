//! Shelf-capacity capability contract tests.
//!
//! ADR 0004: the service measures, the capability decides. These pin the
//! decision half -- in particular that an ambiguous measurement is never
//! reported as a violation.
//!
//! `shelf-capacity` runs as a template (#289): every fixture runs it as a
//! run does (the measured set answered) beside the implementation it
//! replaced (`axioval_rules::reference::ShelfCapacity`) and holds it to
//! that implementation's whole outside contract.
#![allow(missing_docs)]

use std::sync::{Arc, Mutex};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, LinearInterval, LinearQuantityError,
    LinearQuantityEvidence, LinearQuantityRequest, LinearQuantityService,
    LinearQuantityServiceHandle, NotEvaluatedReason, ServiceRegistry,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity as RuleSeverity};
use axioval_ir::{Evidence, ObjectId, RuleId};
use axioval_rules::ShelfCapacity;
use axioval_rules::reference::ShelfCapacity as Reference;

mod common;

use common::{Model, id, unevaluated};

fn rule_with(minimum: ParameterValue) -> CompiledRule {
    let number = |value: f64| ParameterValue::Number { value };
    CompiledRule {
        id: RuleId::new("shelf").unwrap(),
        capability: "axioval:capability.shelf-capacity".into(),
        severity: RuleSeverity::Warning,
        selector: Selector::EntityType {
            object_type: "space".into(),
            include_subtypes: false,
        },
        parameters: [
            ("minimum_running_metres", minimum),
            // A physically realisable arrangement; the measured run is
            // stubbed, so these only need to pass ShelfGeometry validation.
            ("shelf_depth_metres", number(0.4)),
            ("horizontal_spacing_metres", number(0.3)),
            ("vertical_spacing_metres", number(0.35)),
            ("bottom_elevation_metres", number(0.1)),
            ("top_elevation_metres", number(2.0)),
            ("door_clearance_metres", number(0.9)),
            // Doors reach their spaces through `bounds`, forward.
            ("access_path", common::strings(&["bounds:forward"])),
            ("door_selector", common::selector(common::kind("door"))),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect(),
    }
}

fn rule() -> CompiledRule {
    rule_with(ParameterValue::Number { value: 10.0 })
}

#[derive(Clone, Copy)]
enum Answer {
    Measured(LinearInterval),
    Failed(LinearQuantityError),
    /// Evidence that is not exact -- an adapter trying to launder an estimate.
    Inexact(LinearInterval),
}

struct StubQuantities(Answer);

impl LinearQuantityService for StubQuantities {
    fn measure_linear_quantity(
        &self,
        request: &LinearQuantityRequest,
    ) -> Result<LinearQuantityEvidence, LinearQuantityError> {
        match &self.0 {
            Answer::Failed(error) => Err(*error),
            // A room tall enough for the 2 m shelving.
            Answer::Measured(interval) => LinearQuantityEvidence::try_new(
                request.clone(),
                *interval,
                Evidence::exact(common::source(), "shelf:run"),
            )
            .map(|evidence| evidence.with_clear_height(LinearInterval::exact(3.0).unwrap())),
            Answer::Inexact(interval) => LinearQuantityEvidence::try_new(
                request.clone(),
                *interval,
                Evidence {
                    source: common::source(),
                    locator: "shelf:estimate".into(),
                    exact: false,
                },
            ),
        }
    }
}

/// Runs `rule` over `model` as the template, as a run does, and as the
/// implementation it replaced, with `service` (if any) measuring; holds
/// the template to the whole contract and returns its evaluation.
fn run(
    model: Model,
    rule: &CompiledRule,
    service: Option<&dyn Fn() -> Arc<dyn LinearQuantityService>>,
) -> CapabilityEvaluation {
    model.holding_contract(
        &ShelfCapacity,
        &Reference,
        rule,
        |services: &mut ServiceRegistry| {
            if let Some(service) = service {
                services
                    .register(LinearQuantityServiceHandle::new(service()))
                    .unwrap();
            }
        },
        &[],
        0.0,
    )
}

fn lone_store() -> Model {
    Model::default().object("store", "space")
}

fn evaluate_with(answer: Answer, rule: &CompiledRule) -> CapabilityEvaluation {
    run(
        lone_store(),
        rule,
        Some(&|| Arc::new(StubQuantities(answer))),
    )
}

fn messages(evaluation: &CapabilityEvaluation) -> Vec<String> {
    evaluation
        .findings()
        .iter()
        .map(|finding| finding.message.clone())
        .chain(
            evaluation
                .not_evaluated_outcomes()
                .iter()
                .map(|outcome| outcome.message().to_owned()),
        )
        .collect()
}

#[test]
fn measurement_clearing_the_minimum_is_not_a_finding() {
    let outcome = evaluate_with(
        Answer::Measured(LinearInterval::exact(12.0).unwrap()),
        &rule(),
    );
    assert!(outcome.findings().is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

#[test]
fn measurement_below_the_minimum_is_a_finding_carrying_its_evidence() {
    let outcome = evaluate_with(
        Answer::Measured(LinearInterval::exact(4.0).unwrap()),
        &rule(),
    );
    assert_eq!(
        messages(&outcome),
        ["shelf running metres 4.000 below required 10.000"]
    );
    let finding = &outcome.findings()[0];
    assert_eq!(finding.evidence.len(), 1, "a finding must carry its proof");
    assert!(finding.evidence[0].exact);
}

/// The reason the measurement is an interval: an approximate range that spans
/// the minimum has not been shown to fail. Reporting a violation there would
/// assert more than was measured.
#[test]
fn measurement_straddling_the_minimum_is_not_evaluated_rather_than_a_violation() {
    let straddling = LinearInterval::try_new(9.0, 11.0).unwrap();
    let outcome = evaluate_with(Answer::Measured(straddling), &rule());
    assert!(
        outcome.findings().is_empty(),
        "an ambiguous measurement must not become a violation"
    );
    assert_eq!(
        unevaluated(&outcome),
        [("store".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert_eq!(
        messages(&outcome),
        ["measured shelf length between 9 m and 11 m spans the required minimum 10.000"]
    );
}

/// A range entirely below the minimum is a genuine failure even though it is
/// approximate -- fail-closed must not mean "never decide".
#[test]
fn interval_entirely_below_the_minimum_is_still_a_finding() {
    let below = LinearInterval::try_new(2.0, 3.0).unwrap();
    let outcome = evaluate_with(Answer::Measured(below), &rule());
    assert_eq!(
        messages(&outcome),
        ["shelf running metres 3.000 below required 10.000"]
    );
}

#[test]
fn inexact_evidence_is_refused_by_the_service_contract() {
    let outcome = evaluate_with(
        Answer::Inexact(LinearInterval::exact(4.0).unwrap()),
        &rule(),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        unevaluated(&outcome),
        [("store".into(), NotEvaluatedReason::InvalidEvidence)]
    );
    assert_eq!(
        messages(&outcome),
        [LinearQuantityError::InexactEvidence.to_string()]
    );
}

#[test]
fn unavailable_measurement_is_not_a_pass() {
    let outcome = evaluate_with(Answer::Failed(LinearQuantityError::Unavailable), &rule());
    assert!(outcome.findings().is_empty());
    assert_eq!(
        unevaluated(&outcome),
        [("store".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert_eq!(
        messages(&outcome),
        [LinearQuantityError::Unavailable.to_string()]
    );
}

#[test]
fn missing_service_is_neither_a_pass_nor_a_violation() {
    let outcome = run(lone_store(), &rule(), None);
    assert!(outcome.findings().is_empty());
    assert_eq!(
        unevaluated(&outcome),
        [("store".into(), NotEvaluatedReason::MissingService)]
    );
    assert_eq!(
        messages(&outcome),
        ["linear-quantity service is not registered"]
    );
}

#[test]
fn non_numeric_or_negative_minimum_is_an_invalid_declaration() {
    for bad in [
        ParameterValue::String {
            value: "ten".into(),
        },
        ParameterValue::Number { value: -1.0 },
        ParameterValue::Number {
            value: f64::INFINITY,
        },
    ] {
        let outcome = evaluate_with(
            Answer::Measured(LinearInterval::exact(1.0).unwrap()),
            &rule_with(bad),
        );
        assert!(outcome.findings().is_empty());
        assert_eq!(
            unevaluated(&outcome),
            [("store".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
        assert_eq!(
            messages(&outcome),
            ["shelf capacity minimum must be a finite, non-negative number"]
        );
    }
}

/// Geometry is a measurement input, not a threshold. An impossible arrangement
/// is a declaration defect, and must not reach an adapter.
#[test]
fn impossible_shelf_geometry_is_an_invalid_declaration() {
    for (parameter, value) in [
        ("top_elevation_metres", 0.0),
        ("shelf_depth_metres", 0.0),
        ("door_clearance_metres", -0.1),
        ("vertical_spacing_metres", f64::NAN),
    ] {
        let mut rule = rule();
        rule.parameters
            .insert(parameter.into(), ParameterValue::Number { value });
        let outcome = evaluate_with(
            Answer::Measured(LinearInterval::exact(50.0).unwrap()),
            &rule,
        );
        assert!(outcome.findings().is_empty());
        assert_eq!(
            messages(&outcome),
            ["shelf geometry parameters are missing or not physically realisable"],
            "{parameter}"
        );
        assert_eq!(
            unevaluated(&outcome),
            [("store".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

/// Answers every request with a length that falls by 10 m per door, a
/// clear height, and records the doors it was asked about.
struct Doors {
    height: LinearInterval,
    asked: Arc<Mutex<Vec<Vec<ObjectId>>>>,
}

impl LinearQuantityService for Doors {
    fn measure_linear_quantity(
        &self,
        request: &LinearQuantityRequest,
    ) -> Result<LinearQuantityEvidence, LinearQuantityError> {
        self.asked.lock().unwrap().push(request.doors().to_vec());
        let count = f64::from(u32::try_from(request.doors().len()).unwrap());
        LinearQuantityEvidence::try_new(
            request.clone(),
            LinearInterval::exact(20.0 - 10.0 * count)?,
            Evidence::exact(common::source(), "shelf:run"),
        )
        .map(|evidence| evidence.with_clear_height(self.height))
    }
}

/// Space `store` with door `d1` on its boundary and door `d2` elsewhere.
fn store() -> Model {
    Model::default()
        .object("store", "space")
        .object("other", "space")
        .object("d1", "door")
        .object("d2", "door")
        .edge("bounds", "d1", "store")
        .edge("bounds", "d2", "other")
}

fn with_doors(
    model: Model,
    rule: &CompiledRule,
    height: LinearInterval,
) -> (CapabilityEvaluation, Vec<Vec<ObjectId>>) {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let recorded = asked.clone();
    let outcome = run(
        model,
        rule,
        Some(&move || {
            Arc::new(Doors {
                height,
                asked: recorded.clone(),
            })
        }),
    );
    let mut asked = asked.lock().unwrap().clone();
    asked.sort();
    asked.dedup();
    (outcome, asked)
}

/// The doors `access_path` reaches a space from travel in its request; a
/// door of another space does not.
#[test]
fn the_doors_of_each_space_are_sent_with_its_request() {
    let (outcome, asked) = with_doors(store(), &rule(), LinearInterval::exact(3.0).unwrap());
    assert_eq!(asked, vec![vec![id("d1")], vec![id("d2")]]);
    assert!(outcome.findings().is_empty(), "{:?}", outcome.findings());
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

/// A run asks the service once per space, however many values and rules
/// read its shelving: `shelf_length` and `shelf_clear_height` share one
/// request.
#[test]
fn a_run_asks_once_per_space() {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let recorded = asked.clone();
    let outcome = store().evaluate_measured(&ShelfCapacity, &rule(), move |services| {
        services
            .register(LinearQuantityServiceHandle::new(Arc::new(Doors {
                height: LinearInterval::exact(3.0).unwrap(),
                asked: recorded.clone(),
            })))
            .unwrap();
    });
    assert!(outcome.findings().is_empty(), "{:?}", outcome.findings());
    assert_eq!(asked.lock().unwrap().len(), 2, "one request per space");
}

/// A shortfall relates the doors whose clearances were taken out.
#[test]
fn a_shortfall_relates_the_doors_of_the_space() {
    let model = store().edge("bounds", "d2", "store");
    let (outcome, _) = with_doors(model, &rule(), LinearInterval::exact(3.0).unwrap());
    assert_eq!(
        messages(&outcome),
        ["shelf running metres 0.000 below required 10.000"]
    );
    let finding = &outcome.findings()[0];
    assert_eq!(finding.related, vec![id("d1"), id("d2")]);
}

/// A space under the shelving's top elevation is too low for it, whatever
/// length fits.
#[test]
fn a_space_lower_than_the_shelving_is_too_low() {
    let (outcome, _) = with_doors(store(), &rule(), LinearInterval::exact(1.5).unwrap());
    assert_eq!(
        messages(&outcome),
        [
            "space too low for the shelving: clear height 1.5 m below the shelving's top \
             elevation 2 m",
            "space too low for the shelving: clear height 1.5 m below the shelving's top \
             elevation 2 m",
        ]
    );
    assert!(outcome.findings()[0].related.is_empty());

    // A height that may or may not reach 2 m decides nothing.
    let (outcome, _) = with_doors(store(), &rule(), LinearInterval::try_new(1.9, 2.1).unwrap());
    assert!(outcome.findings().is_empty());
    assert_eq!(
        messages(&outcome),
        [
            "clear height between 1.9 m and 2.1 m may or may not reach the shelving's top \
             elevation 2 m",
            "clear height between 1.9 m and 2.1 m may or may not reach the shelving's top \
             elevation 2 m",
        ]
    );
}

/// A space both too low and too short is two findings, one per check.
#[test]
fn a_space_too_low_and_too_short_is_found_twice() {
    let model = store().edge("bounds", "d2", "store");
    let (outcome, _) = with_doors(model, &rule(), LinearInterval::exact(1.5).unwrap());
    let store: Vec<_> = common::findings(&outcome)
        .into_iter()
        .filter(|(subject, _)| subject.ends_with("store"))
        .map(|(_, message)| message)
        .collect();
    assert_eq!(
        store,
        [
            "space too low for the shelving: clear height 1.5 m below the shelving's top \
             elevation 2 m",
            "shelf running metres 0.000 below required 10.000",
        ]
    );
}

/// A door whose spaces cannot be read might open into any space, so no
/// space's shelving is measured without it.
#[test]
fn a_door_with_unreadable_spaces_leaves_every_space_not_evaluated() {
    let mut rule = rule();
    rule.parameters
        .insert("access_path".into(), common::strings(&["unknown:forward"]));
    let (outcome, _) = with_doors(store(), &rule, LinearInterval::exact(3.0).unwrap());
    assert!(outcome.findings().is_empty());
    assert_eq!(unevaluated(&outcome).len(), 2);
    assert!(
        outcome
            .not_evaluated_outcomes()
            .iter()
            .all(|outcome| outcome.reason() == &NotEvaluatedReason::IncompleteEvidence)
    );
    assert!(
        outcome.not_evaluated_outcomes()[0]
            .message()
            .starts_with("the doors and openings of test:model/"),
        "{}",
        outcome.not_evaluated_outcomes()[0].message()
    );
}

/// A door whose kind the selector cannot decide, reaching the space, might
/// take shelving away: the space is left open, as the selection is bound
/// into the measured value with its undecided objects.
#[test]
fn a_door_of_undecided_kind_leaves_its_space_open() {
    let mut rule = rule();
    rule.parameters.insert(
        "door_selector".into(),
        common::selector(Selector::AllOf {
            operands: vec![
                common::kind("door"),
                Selector::Property {
                    property_set: Some("p".into()),
                    property: "Fire".into(),
                    operator: axioval_ir::contract::ComparisonOperator::Exists,
                    value: None,
                    case_sensitive: true,
                    trim: false,
                    quantifier: None,
                    precision: None,
                },
            ],
        }),
    );
    let model =
        store()
            .unreadable("d1")
            .value("d2", "p", "Fire", axioval_ir::PropertyValue::Boolean(true));
    let (outcome, _) = with_doors(model, &rule, LinearInterval::exact(3.0).unwrap());
    assert_eq!(
        unevaluated(&outcome),
        [("store".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

/// Without `access_path` nothing says where the doors are.
#[test]
fn a_rule_without_an_access_path_is_an_invalid_declaration() {
    let mut rule = rule();
    rule.parameters.remove("access_path");
    let outcome = evaluate_with(
        Answer::Measured(LinearInterval::exact(12.0).unwrap()),
        &rule,
    );
    assert_eq!(
        unevaluated(&outcome),
        [("store".into(), NotEvaluatedReason::InvalidDeclaration)]
    );
    assert_eq!(
        messages(&outcome),
        [
            "shelf-capacity: `door_selector`, `opening_selector` and `space_selector` need \
             `access_path`"
        ]
    );
    rule.parameters.remove("door_selector");
    let outcome = evaluate_with(
        Answer::Measured(LinearInterval::exact(12.0).unwrap()),
        &rule,
    );
    assert_eq!(
        messages(&outcome),
        ["shelf-capacity: parameter `access_path` is required"]
    );
}

/// An access path the traversal reader refuses is refused as the
/// capability refused it, through the measured value.
#[test]
fn a_malformed_access_path_is_an_invalid_declaration() {
    let mut rule = rule();
    rule.parameters
        .insert("access_path".into(), common::strings(&["bounds:sideways"]));
    let outcome = evaluate_with(
        Answer::Measured(LinearInterval::exact(12.0).unwrap()),
        &rule,
    );
    assert_eq!(
        unevaluated(&outcome),
        [("store".into(), NotEvaluatedReason::InvalidDeclaration)]
    );
    assert!(
        messages(&outcome)[0].starts_with("shelf-capacity: "),
        "{:?}",
        messages(&outcome)
    );
}

/// An estimated run never becomes an exact measured value: the service
/// contract refuses it, so it is not measured at all. A measured run is
/// exact as its evidence.
#[test]
fn an_estimated_run_is_never_measured_exactly() {
    let name = "shelf_length;depth=0.4;horizontal=0.3;vertical=0.35;bottom=0.1;top=2;\
                clearance=0.9;doors=door;access=bounds:forward";
    let read = |answer: Answer| {
        let (project, mut services) = lone_store().services();
        services
            .register(LinearQuantityServiceHandle::new(Arc::new(StubQuantities(
                answer,
            ))))
            .unwrap();
        common::measured_cited(&services, &project, &id("store"), name)
    };
    let run = LinearInterval::exact(4.0).unwrap();
    assert!(read(Answer::Inexact(run)).is_err());
    assert_eq!(read(Answer::Measured(run)), Ok(Some(((4.0, 4.0), true))));
}

/// Generated spaces with random doors, lengths, heights and minimums: the
/// template keeps the replaced implementation's whole contract.
#[test]
fn generated_spaces_hold_the_contract() {
    let mut seed: u64 = 0x05ee_d289;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..40 {
        let mut model = Model::default();
        for space in 0..3 {
            model = model.object(&format!("s{space}"), "space");
        }
        for door in 0..4 {
            let local = format!("d{door}");
            model = model.object(&local, "door");
            for space in 0..3 {
                if next() % 3 == 0 {
                    model = model.edge("bounds", &local, &format!("s{space}"));
                }
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let height = 1.0 + (next() % 200) as f64 / 100.0;
        #[allow(clippy::cast_precision_loss)]
        let minimum = (next() % 30) as f64;
        let rule = rule_with(ParameterValue::Number { value: minimum });
        let interval = if next() % 2 == 0 {
            LinearInterval::exact(height).unwrap()
        } else {
            LinearInterval::try_new(height - 0.2, height + 0.2).unwrap()
        };
        with_doors(model, &rule, interval);
    }
}

/// The rule forked from the template, an `expression` rule passing the
/// rule's selectors and arrangement to the measured values as the template
/// does (`doors=@door_selector`), reaches the template's verdicts. It
/// reports a space once where the template reports each failed check
/// (divergence D1), so outcomes are compared uncounted.
#[test]
fn the_forked_rule_passes_the_selectors_and_reaches_the_verdicts() {
    use axioval_rules::templates::{Fork, fork};
    let forked = fork(&ShelfCapacity, &rule()).unwrap();
    // The selector and the arrangement travel with the fork, the unstated
    // opening and space selectors do not.
    for name in [
        "door_selector",
        "access_path",
        "shelf_depth_metres",
        "door_clearance_metres",
    ] {
        assert!(forked.carried.contains_key(name), "{name}");
    }
    assert!(!forked.carried.contains_key("opening_selector"));
    let requirement = serde_json::to_string(&forked.requirement).unwrap();
    assert!(
        requirement.contains("doors=@door_selector"),
        "{requirement}"
    );
    assert!(!requirement.contains("@opening_selector"), "{requirement}");
    for (model, height) in [
        (store(), LinearInterval::exact(3.0).unwrap()),
        (
            store().edge("bounds", "d2", "store"),
            LinearInterval::exact(3.0).unwrap(),
        ),
        (store(), LinearInterval::exact(1.5).unwrap()),
        (store(), LinearInterval::try_new(1.9, 2.1).unwrap()),
    ] {
        let mut expression_rule = rule();
        expression_rule.capability = Fork::CAPABILITY.into();
        expression_rule.parameters = forked.parameters();
        let service = move || -> Arc<dyn LinearQuantityService> {
            Arc::new(Doors {
                height,
                asked: Arc::new(Mutex::new(Vec::new())),
            })
        };
        let (template, _) = with_doors(model.clone(), &rule(), height);
        let fork = model.evaluate_measured(
            &axioval_rules::ExpressionRequirement,
            &expression_rule,
            |services| {
                services
                    .register(LinearQuantityServiceHandle::new(service()))
                    .unwrap();
            },
        );
        common::expressions::assert_parity_uncounted(
            "axioval:capability.shelf-capacity",
            &template,
            &fork,
        );
        // The fork's finding on a short space relates the doors too.
        for finding in fork.findings() {
            if finding.message.contains("shelf_length") {
                assert!(!finding.related.is_empty(), "{finding:?}");
            }
        }
    }
}

/// `shelf_clear_height` reaching the shelving's top and `shelf_length` the
/// minimum reach `shelf-capacity`'s verdicts, from the same request with the
/// same doors, the doors named by source kind.
mod as_expressions {
    use super::*;
    use common::expressions::{and, assert_parity_uncounted, at_least, graded, m, measured};
    use serde_json::Value;

    const SHELVING: &str = "depth=0.4;horizontal=0.3;vertical=0.35;bottom=0.1;top=2;\
                            clearance=0.9;doors=door";

    fn requirement(access: &str, minimum: f64) -> Value {
        let value = |name: &str| measured(&format!("{name};{SHELVING};access={access}"));
        and(vec![
            at_least(value("shelf_clear_height"), m(2.0)),
            at_least(value("shelf_length"), m(minimum)),
        ])
    }

    fn parity(
        model: fn() -> Model,
        service: &dyn Fn() -> Arc<dyn LinearQuantityService>,
        capability: &CompiledRule,
        access: &str,
        minimum: f64,
    ) {
        let found = model().evaluate_with(&Reference, capability, |services| {
            services
                .register(LinearQuantityServiceHandle::new(service()))
                .unwrap();
        });
        let rewritten = model().evaluate_measured(
            &axioval_rules::ExpressionRequirement,
            &graded(
                capability.selector.clone(),
                &requirement(access, minimum),
                RuleSeverity::Warning,
            ),
            |services| {
                services
                    .register(LinearQuantityServiceHandle::new(service()))
                    .unwrap();
            },
        );
        // The capability reports each short door, the rewrite the space once
        // (divergence D1).
        assert_parity_uncounted("axioval:capability.shelf-capacity", &found, &rewritten);
    }

    #[test]
    fn the_running_metres_and_the_clear_height_reach_the_verdicts() {
        let answers: [Answer; 6] = [
            Answer::Measured(LinearInterval::exact(12.0).unwrap()),
            Answer::Measured(LinearInterval::exact(4.0).unwrap()),
            Answer::Measured(LinearInterval::try_new(9.0, 11.0).unwrap()),
            Answer::Measured(LinearInterval::try_new(2.0, 3.0).unwrap()),
            Answer::Inexact(LinearInterval::exact(4.0).unwrap()),
            Answer::Failed(LinearQuantityError::Unavailable),
        ];
        for answer in answers {
            for minimum in [10.0, 3.0] {
                parity(
                    lone_store,
                    &|| Arc::new(StubQuantities(answer)),
                    &rule_with(ParameterValue::Number { value: minimum }),
                    "bounds:forward",
                    minimum,
                );
            }
        }
    }

    #[test]
    fn the_doors_of_each_space_and_its_height_reach_the_verdicts() {
        let heights: [LinearInterval; 3] = [
            LinearInterval::exact(3.0).unwrap(),
            LinearInterval::exact(1.5).unwrap(),
            LinearInterval::try_new(1.9, 2.1).unwrap(),
        ];
        for height in heights {
            let doors = move || -> Arc<dyn LinearQuantityService> {
                Arc::new(Doors {
                    height,
                    asked: Arc::new(Mutex::new(Vec::new())),
                })
            };
            parity(store, &doors, &rule(), "bounds:forward", 10.0);
            parity(
                || store().edge("bounds", "d2", "store"),
                &doors,
                &rule(),
                "bounds:forward",
                10.0,
            );
        }
    }

    #[test]
    fn doors_with_unreadable_spaces_leave_every_space_open_alike() {
        let mut capability = rule();
        capability
            .parameters
            .insert("access_path".into(), common::strings(&["unknown:forward"]));
        parity(
            store,
            &|| {
                Arc::new(Doors {
                    height: LinearInterval::exact(3.0).unwrap(),
                    asked: Arc::new(Mutex::new(Vec::new())),
                })
            },
            &capability,
            "unknown:forward",
            10.0,
        );
    }
}
