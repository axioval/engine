//! Exact property-to-property comparison capability tests.
#![allow(missing_docs)]

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use axioval_engine::{
    CapabilityRegistry, CompiledRule, CompletePropertyAbsenceEvidence,
    CompleteRelationshipSelection, PropertyRequest, PropertyResolution, PropertyResolutionError,
    PropertyResolutionService, PropertyResolutionServiceHandle, RelationshipQuery,
    RelationshipSelectionError, RelationshipSelectionRequest, RelationshipSelectionService,
    RelationshipSelectionServiceHandle, ResolvedProperty, RuleCapability, RuleContext,
    SemanticRelationship, ServiceRegistry, TraversalDirection,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity as RuleSeverity};
use axioval_ir::{
    Evidence, NotEvaluatedReason, Object, ObjectId, Project, Property, PropertyValue,
    QuantityDimension, RuleId, SourceId,
};
use axioval_rules::{PropertyComparison, register_builtins};

fn source(name: &str) -> SourceId {
    SourceId::new("test", name).unwrap()
}
fn object(src: &str, local: &str) -> Object {
    Object::new(ObjectId::new(source(src), local).unwrap(), "checked")
}
fn property(set: &str, name: &str, value: PropertyValue, locator: &str) -> Property {
    Property::new(set, name, value)
        .unwrap()
        .with_evidence(Evidence::exact(source("properties"), locator))
}

#[derive(Default)]
struct Properties {
    values: BTreeMap<ObjectId, Vec<Property>>,
}
impl PropertyResolutionService for Properties {
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        if let Some(item) = self.values.get(request.object_id()).and_then(|items| {
            items.iter().find(|item| {
                item.name == request.property()
                    && request
                        .property_set()
                        .is_none_or(|set| item.property_set == set)
            })
        }) {
            Ok(PropertyResolution::Present(ResolvedProperty::try_new(
                request.clone(),
                item.clone(),
            )?))
        } else {
            Ok(PropertyResolution::Absent(
                CompletePropertyAbsenceEvidence::try_new(
                    request.clone(),
                    Evidence::exact(
                        source("properties"),
                        format!("absence:{}", request.object_id()),
                    ),
                )?,
            ))
        }
    }
}

struct Relationships {
    candidates: Vec<ObjectId>,
    evidence: Vec<Evidence>,
    seen: Arc<Mutex<Vec<RelationshipSelectionRequest>>>,
}
impl RelationshipSelectionService for Relationships {
    fn select(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        self.seen.lock().unwrap().push(request.clone());
        CompleteRelationshipSelection::try_new(
            request.clone(),
            self.candidates.clone(),
            self.evidence.clone(),
        )
    }
}

fn rule(mode: &str, quantifier: &str) -> CompiledRule {
    CompiledRule {
        id: RuleId::new("compare").unwrap(),
        capability: "axioval:capability.property-comparison".into(),
        severity: RuleSeverity::Error,
        selector: Selector::EntityType {
            object_type: "checked".into(),
            include_subtypes: true,
        },
        parameters: BTreeMap::from([
            (
                "compared_selector".into(),
                ParameterValue::Selector {
                    value: Box::new(Selector::EntityType {
                        object_type: "candidate".into(),
                        include_subtypes: true,
                    }),
                },
            ),
            (
                "compared_property".into(),
                ParameterValue::PropertyReference {
                    property: "Compared".into(),
                    property_set: Some("Pset".into()),
                },
            ),
            (
                "target_property".into(),
                ParameterValue::PropertyReference {
                    property: "Target".into(),
                    property_set: Some("Pset".into()),
                },
            ),
            (
                "operator".into(),
                ParameterValue::String {
                    value: "equals".into(),
                },
            ),
            ("factor".into(), ParameterValue::Number { value: 2.0 }),
            (
                "component_mode".into(),
                ParameterValue::String { value: mode.into() },
            ),
            (
                "relationship".into(),
                ParameterValue::String {
                    value: "contains".into(),
                },
            ),
            (
                "direction".into(),
                ParameterValue::String {
                    value: "forward".into(),
                },
            ),
            (
                "follow_chain".into(),
                ParameterValue::Boolean { value: false },
            ),
            (
                "quantifier".into(),
                ParameterValue::String {
                    value: quantifier.into(),
                },
            ),
        ]),
    }
}

fn evaluate(
    project: &Project,
    rule: &CompiledRule,
    properties: Properties,
    relationship: Option<Relationships>,
) -> axioval_engine::CapabilityEvaluation {
    let mut services = ServiceRegistry::new();
    services
        .register(PropertyResolutionServiceHandle::new(Arc::new(properties)))
        .unwrap();
    if let Some(relationship) = relationship {
        services
            .register(RelationshipSelectionServiceHandle::new(Arc::new(
                relationship,
            )))
            .unwrap();
    }
    PropertyComparison.evaluate(
        &RuleContext {
            project,
            services: &services,
        },
        rule,
    )
}

fn evaluate_without_properties(
    project: &Project,
    rule: &CompiledRule,
    relationship: Relationships,
) -> axioval_engine::CapabilityEvaluation {
    let mut services = ServiceRegistry::new();
    services
        .register(RelationshipSelectionServiceHandle::new(Arc::new(
            relationship,
        )))
        .unwrap();
    PropertyComparison.evaluate(
        &RuleContext {
            project,
            services: &services,
        },
        rule,
    )
}

#[test]
fn related_each_uses_target_factor_and_emits_one_checked_object_finding() {
    let checked = object("checked-source", "checked");
    let passing = Object::new(
        ObjectId::new(source("candidates"), "a").unwrap(),
        "candidate",
    );
    let failing = Object::new(
        ObjectId::new(source("candidates"), "b").unwrap(),
        "candidate",
    );
    let project = Project::new(vec![checked.clone(), passing.clone(), failing.clone()]).unwrap();
    let mut values = Properties::default();
    values.values.insert(
        checked.id.clone(),
        vec![property(
            "Pset",
            "Target",
            PropertyValue::Decimal(5.0),
            "target",
        )],
    );
    values.values.insert(
        passing.id.clone(),
        vec![property(
            "Pset",
            "Compared",
            PropertyValue::Decimal(10.0),
            "candidate-a",
        )],
    );
    values.values.insert(
        failing.id.clone(),
        vec![property(
            "Pset",
            "Compared",
            PropertyValue::Decimal(9.0),
            "candidate-b",
        )],
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    let outcome = evaluate(
        &project,
        &rule("related", "each"),
        values,
        Some(Relationships {
            candidates: vec![failing.id.clone(), passing.id.clone()],
            evidence: vec![
                Evidence::exact(source("relations"), "z"),
                Evidence::exact(source("relations"), "a"),
            ],
            seen: seen.clone(),
        }),
    );
    assert_eq!(outcome.findings().len(), 1);
    assert_eq!(outcome.findings()[0].object_id, checked.id);
    assert!(outcome.findings()[0].message.contains("candidates/b"));
    assert_eq!(
        outcome.findings()[0]
            .evidence
            .iter()
            .map(|item| item.locator.as_str())
            .collect::<Vec<_>>(),
        vec!["candidate-b", "target", "a", "z"]
    );
    let requests = seen.lock().unwrap();
    assert_eq!(requests[0].anchor(), &checked.id);
    assert_eq!(
        requests[0].candidate_universe(),
        &[passing.id.clone(), failing.id.clone()]
    );
    assert_eq!(
        requests[0].query(),
        &RelationshipQuery::Related {
            relationship: SemanticRelationship::try_new("contains").unwrap(),
            direction: TraversalDirection::Forward,
            follow_chain: false
        }
    );
}

#[test]
fn incompatible_types_are_not_evaluated_not_findings() {
    let checked = object("checked", "x");
    let candidate = Object::new(
        ObjectId::new(source("candidate"), "y").unwrap(),
        "candidate",
    );
    let project = Project::new(vec![checked.clone(), candidate.clone()]).unwrap();
    let mut values = Properties::default();
    values.values.insert(
        checked.id.clone(),
        vec![property(
            "Pset",
            "Target",
            PropertyValue::String("x".into()),
            "target",
        )],
    );
    values.values.insert(
        candidate.id.clone(),
        vec![property(
            "Pset",
            "Compared",
            PropertyValue::Integer(1),
            "candidate",
        )],
    );
    let outcome = evaluate(
        &project,
        &rule("related", "each"),
        values,
        Some(Relationships {
            candidates: vec![candidate.id.clone()],
            evidence: vec![Evidence::exact(source("relations"), "complete")],
            seen: Arc::new(Mutex::new(Vec::new())),
        }),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::InvalidEvidence
    );
}

#[test]
fn missing_relationship_service_is_explicit() {
    let checked = object("checked", "x");
    let project = Project::new(vec![checked]).unwrap();
    let outcome = evaluate(
        &project,
        &rule("shared", "each"),
        Properties::default(),
        None,
    );
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::MissingService
    );
}

#[test]
fn exact_empty_existential_does_not_require_property_service() {
    let checked = object("checked", "x");
    let candidate = Object::new(
        ObjectId::new(source("candidate"), "y").unwrap(),
        "candidate",
    );
    let project = Project::new(vec![checked.clone(), candidate]).unwrap();
    let outcome = evaluate_without_properties(
        &project,
        &rule("related", "at_least_one"),
        Relationships {
            candidates: vec![],
            evidence: vec![Evidence::exact(source("relations"), "complete")],
            seen: Arc::new(Mutex::new(Vec::new())),
        },
    );
    assert!(outcome.not_evaluated_outcomes().is_empty());
    assert_eq!(outcome.findings().len(), 1);
    assert_eq!(outcome.findings()[0].object_id, checked.id);
}

#[test]
fn empty_at_least_one_is_a_finding_backed_by_completeness_evidence() {
    let checked = object("checked", "x");
    let candidate = Object::new(
        ObjectId::new(source("candidate"), "y").unwrap(),
        "candidate",
    );
    let project = Project::new(vec![checked.clone(), candidate]).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let outcome = evaluate(
        &project,
        &rule("related", "at_least_one"),
        Properties::default(),
        Some(Relationships {
            candidates: vec![],
            evidence: vec![Evidence::exact(source("relations"), "complete")],
            seen,
        }),
    );
    assert_eq!(outcome.findings().len(), 1);
    assert_eq!(outcome.findings()[0].object_id, checked.id);
    assert_eq!(outcome.findings()[0].evidence[0].locator, "complete");
}

#[test]
fn missing_candidate_property_is_its_own_existential_finding() {
    let checked = object("checked", "x");
    let candidate = Object::new(
        ObjectId::new(source("candidate"), "missing").unwrap(),
        "candidate",
    );
    let project = Project::new(vec![checked.clone(), candidate.clone()]).unwrap();
    let mut values = Properties::default();
    values.values.insert(
        checked.id.clone(),
        vec![property(
            "Pset",
            "Target",
            PropertyValue::Decimal(5.0),
            "target",
        )],
    );
    let outcome = evaluate(
        &project,
        &rule("related", "at_least_one"),
        values,
        Some(Relationships {
            candidates: vec![candidate.id.clone()],
            evidence: vec![Evidence::exact(source("relations"), "complete")],
            seen: Arc::new(Mutex::new(Vec::new())),
        }),
    );
    assert!(outcome.not_evaluated_outcomes().is_empty());
    assert_eq!(outcome.findings().len(), 1);
    assert_eq!(outcome.findings()[0].object_id, candidate.id);
    assert!(outcome.findings()[0].message.contains("absent"));
}

#[test]
fn each_preserves_known_failures_when_another_candidate_is_uncertain() {
    let checked = object("checked", "x");
    let failing = Object::new(
        ObjectId::new(source("candidate"), "failing").unwrap(),
        "candidate",
    );
    let uncertain = Object::new(
        ObjectId::new(source("candidate"), "uncertain").unwrap(),
        "candidate",
    );
    let project = Project::new(vec![checked.clone(), failing.clone(), uncertain.clone()]).unwrap();
    let mut values = Properties::default();
    values.values.insert(
        checked.id.clone(),
        vec![property(
            "Pset",
            "Target",
            PropertyValue::Decimal(5.0),
            "target",
        )],
    );
    values.values.insert(
        failing.id.clone(),
        vec![property(
            "Pset",
            "Compared",
            PropertyValue::Decimal(9.0),
            "failing",
        )],
    );
    values.values.insert(
        uncertain.id.clone(),
        vec![property(
            "Pset",
            "Compared",
            PropertyValue::String("nine".into()),
            "uncertain",
        )],
    );
    let outcome = evaluate(
        &project,
        &rule("related", "each"),
        values,
        Some(Relationships {
            candidates: vec![uncertain.id, failing.id],
            evidence: vec![Evidence::exact(source("relations"), "complete")],
            seen: Arc::new(Mutex::new(Vec::new())),
        }),
    );
    assert_eq!(outcome.findings().len(), 1);
    assert_eq!(outcome.not_evaluated_outcomes().len(), 1);
}

#[test]
fn existential_aggregate_keeps_all_property_proof() {
    let checked = object("checked", "x");
    let a = Object::new(
        ObjectId::new(source("candidate"), "a").unwrap(),
        "candidate",
    );
    let b = Object::new(
        ObjectId::new(source("candidate"), "b").unwrap(),
        "candidate",
    );
    let project = Project::new(vec![checked.clone(), a.clone(), b.clone()]).unwrap();
    let mut values = Properties::default();
    values.values.insert(
        checked.id.clone(),
        vec![property(
            "Pset",
            "Target",
            PropertyValue::Decimal(5.0),
            "target",
        )],
    );
    values.values.insert(
        a.id.clone(),
        vec![property(
            "Pset",
            "Compared",
            PropertyValue::Decimal(8.0),
            "candidate-a",
        )],
    );
    values.values.insert(
        b.id.clone(),
        vec![property(
            "Pset",
            "Compared",
            PropertyValue::Decimal(9.0),
            "candidate-b",
        )],
    );
    let outcome = evaluate(
        &project,
        &rule("related", "at_least_one"),
        values,
        Some(Relationships {
            candidates: vec![a.id, b.id],
            evidence: vec![Evidence::exact(source("relations"), "complete")],
            seen: Arc::new(Mutex::new(Vec::new())),
        }),
    );
    assert_eq!(outcome.findings().len(), 1);
    let locators = outcome.findings()[0]
        .evidence
        .iter()
        .map(|item| item.locator.as_str())
        .collect::<Vec<_>>();
    assert!(locators.contains(&"candidate-a"));
    assert!(locators.contains(&"candidate-b"));
    assert!(locators.contains(&"target"));
}

#[test]
fn quantity_target_factor_overflow_is_not_evaluated() {
    let checked = object("checked", "overflow");
    let project = Project::new(vec![checked.clone()]).unwrap();
    let mut values = Properties::default();
    values.values.insert(
        checked.id.clone(),
        vec![
            property(
                "Pset",
                "Target",
                PropertyValue::Quantity {
                    value: f64::MAX,
                    dimension: QuantityDimension::Length,
                },
                "target",
            ),
            property(
                "Pset",
                "Compared",
                PropertyValue::Quantity {
                    value: f64::MAX,
                    dimension: QuantityDimension::Length,
                },
                "compared",
            ),
        ],
    );
    let mut overflow_rule = rule("checked", "each");
    overflow_rule.parameters.insert(
        "compared_selector".into(),
        ParameterValue::Selector {
            value: Box::new(Selector::All),
        },
    );
    let outcome = evaluate(&project, &overflow_rule, values, None);
    assert!(outcome.findings().is_empty());
    assert_eq!(outcome.not_evaluated_outcomes().len(), 1);
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::InvalidEvidence
    );
}

#[test]
fn builtins_register_property_comparison() {
    assert!(
        register_builtins(CapabilityRegistry::new())
            .unwrap()
            .get("axioval:capability.property-comparison")
            .is_some()
    );
}
