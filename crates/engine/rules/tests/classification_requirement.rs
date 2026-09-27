//! `classification`: requirements a classification selector cannot state,
//! decided over a fake classification service.
#![allow(missing_docs)]

use std::{collections::BTreeMap, sync::Arc};

use axioval_engine::{
    CapabilityEvaluation, ClassificationAssignment, ClassificationError, ClassificationService,
    ClassificationServiceHandle, CompiledRule, RuleCapability, RuleContext, ServiceRegistry,
    SourceSnapshot,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::{NotEvaluatedReason, Object, ObjectId, Project, RuleId, SourceId};
use axioval_rules::ClassificationRequirement;

fn source() -> SourceId {
    SourceId::new("cad", "native-model").unwrap()
}

fn snapshot() -> SourceSnapshot {
    SourceSnapshot::try_new(source(), "r1", "sha256:1").unwrap()
}

#[derive(Debug, PartialEq)]
enum Outcome {
    Meets,
    Fails,
    NotEvaluated(NotEvaluatedReason),
}

fn outcome(evaluation: &CapabilityEvaluation) -> Outcome {
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

fn flag(name: &'static str) -> (&'static str, ParameterValue) {
    (name, ParameterValue::Boolean { value: true })
}

struct Classifications(Vec<ClassificationAssignment>, Vec<SourceSnapshot>);
impl ClassificationService for Classifications {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.1
    }
    fn classifications(
        &self,
        _: &ObjectId,
    ) -> Result<Vec<ClassificationAssignment>, ClassificationError> {
        Ok(self.0.clone())
    }
}

/// Assignments as `(system, codes leaf first)`.
fn classified(assignments: &[(Option<&str>, &[&str])]) -> ServiceRegistry {
    let assignments = assignments
        .iter()
        .map(|(system, codes)| ClassificationAssignment {
            system: system.map(str::to_owned),
            codes: codes.iter().map(|code| Some((*code).to_owned())).collect(),
        })
        .collect();
    let mut services = ServiceRegistry::new();
    services
        .register(ClassificationServiceHandle::new(Arc::new(Classifications(
            assignments,
            vec![snapshot()],
        ))))
        .unwrap();
    services
}

fn run(services: &ServiceRegistry, parameters: &[(&str, ParameterValue)]) -> Outcome {
    let project = Project::new(vec![Object::new(
        ObjectId::new(source(), "wall-1").unwrap(),
        "wall",
    )])
    .unwrap();
    let rule = CompiledRule {
        id: RuleId::new("rule").unwrap(),
        capability: ClassificationRequirement.id().into(),
        severity: Severity::Error,
        selector: Selector::All,
        parameters: parameters
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect::<BTreeMap<_, _>>(),
    };
    outcome(&ClassificationRequirement.evaluate(
        &RuleContext {
            project: &project,
            services,
        },
        &rule,
    ))
}

#[test]
fn codes_match_the_whole_chain_and_systems_by_literal_or_pattern() {
    let services = classified(&[(Some("Uniclass"), &["EF_25_10_25", "EF_25_10"])]);
    let check = |parameters: &[(&str, ParameterValue)]| run(&services, parameters);
    assert_eq!(check(&[]), Outcome::Meets);
    assert_eq!(check(&[list("codes", &["EF_25_10"])]), Outcome::Meets);
    assert_eq!(check(&[list("codes", &["EF_25"])]), Outcome::Fails);
    assert_eq!(
        check(&[list("code_patterns", &["EF_25.*"])]),
        Outcome::Meets
    );
    assert_eq!(check(&[list("systems", &["Uniclass"])]), Outcome::Meets);
    assert_eq!(
        check(&[list("system_patterns", &["Uni.*"])]),
        Outcome::Meets
    );
    assert_eq!(check(&[list("systems", &["DIN 276"])]), Outcome::Fails);
    assert_eq!(
        check(&[list("codes", &["EF_25_10"]), flag("prohibited")]),
        Outcome::Fails
    );
    assert_eq!(
        check(&[list("codes", &["X"]), flag("prohibited")]),
        Outcome::Meets
    );
}

#[test]
fn one_assignment_meets_the_system_and_the_code_together() {
    // As a classification selector decides: a code in another system does
    // not meet a system-qualified requirement.
    let services = classified(&[(Some("Uniclass"), &["EF_25"]), (Some("DIN 276"), &["330"])]);
    assert_eq!(
        run(
            &services,
            &[list("systems", &["Uniclass"]), list("codes", &["330"])]
        ),
        Outcome::Fails
    );
    assert_eq!(
        run(
            &services,
            &[list("systems", &["DIN 276"]), list("codes", &["330"])]
        ),
        Outcome::Meets
    );
}

#[test]
fn an_unclassified_object_fails_unless_optional() {
    let services = classified(&[]);
    assert_eq!(run(&services, &[]), Outcome::Fails);
    assert_eq!(run(&services, &[flag("optional")]), Outcome::Meets);
    assert_eq!(run(&services, &[flag("prohibited")]), Outcome::Meets);
}

#[test]
fn an_optional_classification_must_be_met_once_the_object_carries_any() {
    let services = classified(&[(Some("Uniclass"), &["EF_25"])]);
    let optional = |system: &str, code: &str| {
        run(
            &services,
            &[
                list("systems", &[system]),
                list("codes", &[code]),
                flag("optional"),
            ],
        )
    };
    assert_eq!(optional("Uniclass", "EF_25"), Outcome::Meets);
    assert_eq!(optional("Uniclass", "EF_30"), Outcome::Fails);
    assert_eq!(optional("DIN 276", "330"), Outcome::Fails);
}

#[test]
fn an_unstated_system_that_could_decide_is_not_evaluated() {
    let services = classified(&[(None, &["21"])]);
    assert_eq!(
        run(&services, &[list("systems", &["Uniclass"])]),
        Outcome::NotEvaluated(NotEvaluatedReason::IncompleteEvidence)
    );
    // It cannot decide a code requirement it does not meet either way.
    assert_eq!(
        run(
            &services,
            &[list("systems", &["Uniclass"]), list("codes", &["99"])]
        ),
        Outcome::Fails
    );
}

#[test]
fn optional_and_prohibited_together_and_bad_patterns_are_invalid() {
    let services = classified(&[]);
    assert_eq!(
        run(&services, &[flag("optional"), flag("prohibited")]),
        Outcome::NotEvaluated(NotEvaluatedReason::InvalidDeclaration)
    );
    assert_eq!(
        run(&services, &[list("code_patterns", &["[a-z-[aeiou]]"])]),
        Outcome::NotEvaluated(NotEvaluatedReason::InvalidDeclaration)
    );
}
