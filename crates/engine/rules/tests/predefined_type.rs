//! `axioval:capability.predefined-type`.
#![allow(missing_docs)]

use std::{collections::BTreeMap, sync::Arc};

use axioval_engine::{
    CompiledRule, CompletePropertyAbsenceEvidence, PropertyRequest, PropertyResolution,
    PropertyResolutionError, PropertyResolutionService, PropertyResolutionServiceHandle,
    ResolvedProperty, RuleCapability, RuleContext, ServiceRegistry, SourceSnapshot,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::{
    Evidence, NotEvaluatedReason, Object, ObjectId, PREDEFINED_TYPE, PREDEFINED_TYPE_SET,
    PREDEFINED_TYPE_USER_DEFINED, Project, Property, PropertyValue, RuleId, SourceId,
};
use axioval_rules::PredefinedTypeRequirement;

fn source() -> SourceId {
    SourceId::new("cad", "native-model").unwrap()
}

/// A source answering the reserved predefined-type set, and nothing else.
struct Fixed(Option<&'static str>, bool, Vec<SourceSnapshot>);
impl PropertyResolutionService for Fixed {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.2
    }
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        assert_eq!(request.property_set(), Some(PREDEFINED_TYPE_SET));
        let value = match request.property() {
            PREDEFINED_TYPE => self.0.map(|text| PropertyValue::String(text.into())),
            PREDEFINED_TYPE_USER_DEFINED => Some(PropertyValue::Boolean(self.1)),
            other => panic!("unexpected {other}"),
        };
        let evidence = Evidence::exact(source(), "native wall type");
        Ok(match value {
            Some(value) => PropertyResolution::Present(ResolvedProperty::try_new(
                request.clone(),
                Property::new(PREDEFINED_TYPE_SET, request.property(), value)
                    .unwrap()
                    .with_evidence(evidence),
            )?),
            None => PropertyResolution::Absent(CompletePropertyAbsenceEvidence::try_new(
                request.clone(),
                evidence,
            )?),
        })
    }
}

#[derive(Debug, PartialEq)]
enum Outcome {
    Meets,
    Fails,
    NotEvaluated(NotEvaluatedReason),
}

fn check(
    value: Option<&'static str>,
    user_defined: bool,
    parameters: &[(&str, ParameterValue)],
) -> Outcome {
    let project = Project::new(vec![Object::new(
        ObjectId::new(source(), "wall-1").unwrap(),
        "wall",
    )])
    .unwrap();
    let snapshot = SourceSnapshot::try_new(source(), "r1", "sha256:1").unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(PropertyResolutionServiceHandle::new(Arc::new(Fixed(
            value,
            user_defined,
            vec![snapshot],
        ))))
        .unwrap();
    let rule = CompiledRule {
        id: RuleId::new("predefined").unwrap(),
        capability: "axioval:capability.predefined-type".into(),
        severity: Severity::Error,
        selector: Selector::All,
        parameters: parameters
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect::<BTreeMap<_, _>>(),
    };
    let evaluation = PredefinedTypeRequirement.evaluate(
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
            Outcome::Fails
        }
        ([], [outcome]) => Outcome::NotEvaluated(outcome.reason().clone()),
        (findings, outcomes) => panic!("{findings:?} {outcomes:?}"),
    }
}

fn list(name: &'static str, values: &[&str]) -> (&'static str, ParameterValue) {
    (
        name,
        ParameterValue::StringList {
            value: values.iter().map(|value| (*value).to_owned()).collect(),
        },
    )
}

fn user_defined() -> (&'static str, ParameterValue) {
    ("user_defined", ParameterValue::Boolean { value: true })
}

#[test]
fn values_compare_exactly() {
    assert_eq!(
        check(Some("SOLIDWALL"), false, &[list("values", &["SOLIDWALL"])]),
        Outcome::Meets
    );
    assert_eq!(
        check(Some("SOLIDWALL"), false, &[list("values", &["solidwall"])]),
        Outcome::Fails
    );
    assert_eq!(
        check(None, false, &[list("values", &["SOLIDWALL"])]),
        Outcome::Fails
    );
}

#[test]
fn patterns_match_the_whole_designation() {
    assert_eq!(
        check(Some("FOOBAR"), true, &[list("patterns", &["FOO.*"])]),
        Outcome::Meets
    );
    assert_eq!(
        check(Some("BAZFOO"), true, &[list("patterns", &["FOO.*"])]),
        Outcome::Fails
    );
}

#[test]
fn user_defined_asks_only_whether_it_is() {
    assert_eq!(
        check(Some("WALDO"), true, &[user_defined()]),
        Outcome::Meets
    );
    assert_eq!(
        check(Some("SHEAR"), false, &[user_defined()]),
        Outcome::Fails
    );
}

#[test]
fn without_a_property_service_nothing_is_decided() {
    let project = Project::new(vec![Object::new(
        ObjectId::new(source(), "wall-1").unwrap(),
        "wall",
    )])
    .unwrap();
    let rule = CompiledRule {
        id: RuleId::new("predefined").unwrap(),
        capability: "axioval:capability.predefined-type".into(),
        severity: Severity::Error,
        selector: Selector::All,
        parameters: BTreeMap::from([list("values", &["X"])])
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    };
    let evaluation = PredefinedTypeRequirement.evaluate(
        &RuleContext {
            project: &project,
            services: &ServiceRegistry::new(),
        },
        &rule,
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        evaluation.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::MissingService
    );
}

#[test]
fn declarations_need_exactly_one_kind_of_requirement() {
    for parameters in [
        vec![],
        vec![user_defined(), list("values", &["X"])],
        vec![list("patterns", &["[a-z-[aeiou]]"])],
    ] {
        assert_eq!(
            check(Some("X"), false, &parameters),
            Outcome::NotEvaluated(NotEvaluatedReason::InvalidDeclaration),
            "{parameters:?}"
        );
    }
}
