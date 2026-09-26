//! `axioval:capability.attribute-value` over a source's attribute service.
#![allow(missing_docs)]

use std::{collections::BTreeMap, sync::Arc};

use axioval_engine::{
    AttributeError, AttributeService, AttributeServiceHandle, AttributeValue, CompiledRule,
    ResolvedAttribute, RuleCapability, RuleContext, ServiceRegistry, SourceSnapshot,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::{
    Evidence, NotEvaluatedReason, Object, ObjectId, Project, PropertyValue, RuleId, SourceId,
};
use axioval_rules::AttributeValueConstraint;

fn source() -> SourceId {
    SourceId::new("cad", "native-model").unwrap()
}

/// One object whose `Name` holds a fixed value; every other name is unknown.
struct OneAttribute(AttributeValue, Vec<SourceSnapshot>);
impl AttributeService for OneAttribute {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.1
    }
    fn attribute(&self, _: &ObjectId, name: &str) -> Result<ResolvedAttribute, AttributeError> {
        if name != "Name" {
            return Err(AttributeError::UnknownAttribute {
                class: "wall".into(),
                attribute: name.into(),
            });
        }
        Ok(ResolvedAttribute {
            value: self.0.clone(),
            evidence: Evidence::exact(source(), "native wall.Name"),
        })
    }
}

#[derive(Debug, PartialEq)]
enum Outcome {
    Meets,
    Fails(String),
    NotEvaluated(NotEvaluatedReason),
}

fn check(value: AttributeValue, attribute: &str, parameters: &[(&str, ParameterValue)]) -> Outcome {
    let project = Project::new(vec![Object::new(
        ObjectId::new(source(), "wall-1").unwrap(),
        "wall",
    )])
    .unwrap();
    let snapshot = SourceSnapshot::try_new(source(), "r1", "sha256:1").unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(AttributeServiceHandle::new(Arc::new(OneAttribute(
            value,
            vec![snapshot],
        ))))
        .unwrap();
    let mut all = BTreeMap::from([(
        "attribute".to_owned(),
        ParameterValue::PropertyReference {
            property: attribute.into(),
            property_set: None,
        },
    )]);
    for (name, value) in parameters {
        all.insert((*name).to_owned(), value.clone());
    }
    let rule = CompiledRule {
        id: RuleId::new("attribute").unwrap(),
        capability: "axioval:capability.attribute-value".into(),
        severity: Severity::Error,
        selector: Selector::All,
        parameters: all,
    };
    let evaluation = AttributeValueConstraint.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        &rule,
    );
    match (evaluation.findings(), evaluation.not_evaluated_outcomes()) {
        ([], []) => Outcome::Meets,
        ([finding], []) => {
            assert!(!finding.evidence.is_empty());
            Outcome::Fails(finding.message.clone())
        }
        ([], [outcome]) => Outcome::NotEvaluated(outcome.reason().clone()),
        (findings, outcomes) => panic!("{findings:?} {outcomes:?}"),
    }
}

fn text(value: &str) -> AttributeValue {
    AttributeValue::Scalar {
        value: PropertyValue::String(value.into()),
        data_type: Some("IFCLABEL".into()),
    }
}

fn values(literals: &[&str]) -> (&'static str, ParameterValue) {
    (
        "values",
        ParameterValue::StringList {
            value: literals.iter().map(|value| (*value).to_owned()).collect(),
        },
    )
}

fn optional() -> (&'static str, ParameterValue) {
    ("optional", ParameterValue::Boolean { value: true })
}

#[test]
fn presence_alone_requires_a_non_empty_value() {
    assert_eq!(check(text("W-1"), "Name", &[]), Outcome::Meets);
    assert_eq!(
        check(AttributeValue::Structured, "Name", &[]),
        Outcome::Meets
    );
    for value in [AttributeValue::Unset, text(""), text("  ")] {
        assert_eq!(
            check(value, "Name", &[]),
            Outcome::Fails("missing required attribute Name".into())
        );
    }
}

#[test]
fn values_compare_as_for_properties() {
    assert_eq!(
        check(text("W-1"), "Name", &[values(&["W-1"])]),
        Outcome::Meets
    );
    assert_eq!(
        check(text("w-1"), "Name", &[values(&["W-1"])]),
        Outcome::Fails("attribute Name is \"w-1\", not one of the required values".into())
    );
}

#[test]
fn optional_attributes_pass_unset_and_check_what_is_there() {
    assert_eq!(
        check(AttributeValue::Unset, "Name", &[values(&["x"]), optional()]),
        Outcome::Meets
    );
    assert!(matches!(
        check(text(""), "Name", &[values(&["x"]), optional()]),
        Outcome::Fails(_)
    ));
}

#[test]
fn references_and_unknown_attributes_are_not_evaluated() {
    assert_eq!(
        check(AttributeValue::Structured, "Name", &[values(&["x"])]),
        Outcome::NotEvaluated(NotEvaluatedReason::InvalidDeclaration)
    );
    assert_eq!(
        check(text("x"), "Colour", &[]),
        Outcome::NotEvaluated(NotEvaluatedReason::InvalidDeclaration)
    );
}

#[test]
fn a_set_qualified_reference_is_an_invalid_declaration() {
    let project = Project::new(vec![]).unwrap();
    let rule = CompiledRule {
        id: RuleId::new("attribute").unwrap(),
        capability: "axioval:capability.attribute-value".into(),
        severity: Severity::Error,
        selector: Selector::All,
        parameters: BTreeMap::from([(
            "attribute".to_owned(),
            ParameterValue::PropertyReference {
                property: "Name".into(),
                property_set: Some("P".into()),
            },
        )]),
    };
    let evaluation = AttributeValueConstraint.evaluate(
        &RuleContext {
            project: &project,
            services: &ServiceRegistry::new(),
        },
        &rule,
    );
    assert_eq!(evaluation.not_evaluated_outcomes().len(), 1);
}
