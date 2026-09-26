//! Classification, material, part-of and entity requirements over fake
//! services, one decision each.
#![allow(missing_docs)]

use std::{collections::BTreeMap, sync::Arc};

use axioval_engine::{
    CapabilityEvaluation, ClassificationAssignment, ClassificationError, ClassificationService,
    ClassificationServiceHandle, CompiledRule, Decomposition, DecompositionError,
    DecompositionService, DecompositionServiceHandle, MaterialError, MaterialService,
    MaterialServiceHandle, ResolvedMaterial, ResolvedWholes, RuleCapability, RuleContext,
    ServiceRegistry, SourceSnapshot, Whole,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::{Evidence, NotEvaluatedReason, Object, ObjectId, Project, RuleId, SourceId};
use axioval_rules::{
    ClassificationRequirement, EntityRequirement, MaterialRequirement, PartOfRequirement,
};

fn source() -> SourceId {
    SourceId::new("cad", "native-model").unwrap()
}

fn snapshot() -> SourceSnapshot {
    SourceSnapshot::try_new(source(), "r1", "sha256:1").unwrap()
}

fn evidence() -> Evidence {
    Evidence::exact(source(), "native")
}

#[derive(Debug, PartialEq)]
enum Outcome {
    Meets,
    Fails,
    NotEvaluated(NotEvaluatedReason),
}

fn run(
    capability: &dyn RuleCapability,
    kind: &str,
    services: &ServiceRegistry,
    parameters: &[(&str, ParameterValue)],
) -> Outcome {
    let project = Project::new(vec![Object::new(
        ObjectId::new(source(), "wall-1").unwrap(),
        kind,
    )])
    .unwrap();
    let rule = CompiledRule {
        id: RuleId::new("rule").unwrap(),
        capability: capability.id().into(),
        severity: Severity::Error,
        selector: Selector::All,
        parameters: parameters
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect::<BTreeMap<_, _>>(),
    };
    outcome(&capability.evaluate(
        &RuleContext {
            project: &project,
            services,
        },
        &rule,
    ))
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

#[test]
fn classification_codes_match_the_whole_chain_and_systems_independently() {
    let services = classified(&[(Some("Uniclass"), &["EF_25_10_25", "EF_25_10"])]);
    let check = |parameters: &[(&str, ParameterValue)]| {
        run(&ClassificationRequirement, "wall", &services, parameters)
    };
    assert_eq!(check(&[]), Outcome::Meets);
    assert_eq!(check(&[list("codes", &["EF_25_10"])]), Outcome::Meets);
    assert_eq!(check(&[list("codes", &["EF_25"])]), Outcome::Fails);
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
fn an_unclassified_object_fails_unless_optional() {
    let services = classified(&[]);
    assert_eq!(
        run(&ClassificationRequirement, "wall", &services, &[]),
        Outcome::Fails
    );
    assert_eq!(
        run(
            &ClassificationRequirement,
            "wall",
            &services,
            &[flag("optional")]
        ),
        Outcome::Meets
    );
}

#[test]
fn an_unstated_system_that_could_decide_is_not_evaluated() {
    let services = classified(&[(None, &["21"])]);
    assert_eq!(
        run(
            &ClassificationRequirement,
            "wall",
            &services,
            &[list("systems", &["Uniclass"])]
        ),
        Outcome::NotEvaluated(NotEvaluatedReason::IncompleteEvidence)
    );
    // It cannot decide a code requirement nothing meets.
    assert_eq!(
        run(
            &ClassificationRequirement,
            "wall",
            &services,
            &[list("systems", &["Uniclass"]), list("codes", &["99"])]
        ),
        Outcome::Fails
    );
}

struct Materials(Option<Vec<&'static str>>, Vec<SourceSnapshot>);
impl MaterialService for Materials {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.1
    }
    fn material(&self, _: &ObjectId) -> Result<Option<ResolvedMaterial>, MaterialError> {
        Ok(self.0.as_ref().map(|names| ResolvedMaterial {
            names: names.iter().map(|name| (*name).to_owned()).collect(),
            evidence: evidence(),
        }))
    }
}

fn material(names: Option<Vec<&'static str>>, parameters: &[(&str, ParameterValue)]) -> Outcome {
    let mut services = ServiceRegistry::new();
    services
        .register(MaterialServiceHandle::new(Arc::new(Materials(
            names,
            vec![snapshot()],
        ))))
        .unwrap();
    run(&MaterialRequirement, "wall", &services, parameters)
}

#[test]
fn materials_match_any_name_and_fail_when_absent() {
    let names = || Some(vec!["Concrete", "Load-bearing"]);
    assert_eq!(material(names(), &[]), Outcome::Meets);
    assert_eq!(
        material(names(), &[list("values", &["Concrete"])]),
        Outcome::Meets
    );
    assert_eq!(
        material(names(), &[list("patterns", &["Con.*"])]),
        Outcome::Meets
    );
    assert_eq!(
        material(names(), &[list("values", &["Steel"])]),
        Outcome::Fails
    );
    assert_eq!(
        material(Some(vec![]), &[list("values", &["Steel"])]),
        Outcome::Fails
    );
    assert_eq!(material(None, &[]), Outcome::Fails);
    assert_eq!(material(None, &[flag("optional")]), Outcome::Meets);
    assert_eq!(material(names(), &[flag("prohibited")]), Outcome::Fails);
    assert_eq!(material(None, &[flag("prohibited")]), Outcome::Meets);
}

struct Wholes(
    Vec<(&'static str, Option<&'static str>)>,
    Vec<SourceSnapshot>,
);
impl DecompositionService for Wholes {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.1
    }
    fn wholes(
        &self,
        _: &ObjectId,
        decomposition: Decomposition,
    ) -> Result<ResolvedWholes, DecompositionError> {
        if decomposition == Decomposition::Grouping {
            return Err(DecompositionError::Ambiguous("two groups".into()));
        }
        Ok(ResolvedWholes {
            wholes: self
                .0
                .iter()
                .map(|(class, predefined)| Whole {
                    class: (*class).to_owned(),
                    predefined_type: predefined.map(str::to_owned),
                    object: None,
                })
                .collect(),
            evidence: evidence(),
        })
    }
}

fn part_of(parameters: &[(&str, ParameterValue)]) -> Outcome {
    let mut services = ServiceRegistry::new();
    services
        .register(DecompositionServiceHandle::new(Arc::new(Wholes(
            vec![
                ("IFCSPACE", None),
                ("IFCBUILDINGSTOREY", Some("BASEMENT")),
                ("IFCBUILDINGSTOREY", Some("GROUND")),
            ],
            vec![snapshot()],
        ))))
        .unwrap();
    run(&PartOfRequirement, "beam", &services, parameters)
}

#[test]
fn the_first_whole_of_the_class_decides() {
    let relation = |value: &str| {
        (
            "relation",
            ParameterValue::String {
                value: value.into(),
            },
        )
    };
    assert_eq!(
        part_of(&[list("classes", &["IFCBUILDINGSTOREY"])]),
        Outcome::Meets
    );
    assert_eq!(part_of(&[list("classes", &["IFCSITE"])]), Outcome::Fails);
    assert_eq!(
        part_of(&[
            list("classes", &["IFCBUILDINGSTOREY"]),
            list("predefined_types", &["BASEMENT"])
        ]),
        Outcome::Meets
    );
    // The nearest storey decides; a farther one does not count.
    assert_eq!(
        part_of(&[
            list("classes", &["IFCBUILDINGSTOREY"]),
            list("predefined_types", &["GROUND"])
        ]),
        Outcome::Fails
    );
    assert_eq!(
        part_of(&[list("class_patterns", &["IFC.*"])]),
        Outcome::Meets
    );
    assert_eq!(
        part_of(&[list("classes", &["IFCSPACE"]), flag("prohibited")]),
        Outcome::Fails
    );
    assert_eq!(
        part_of(&[list("classes", &["IFCZONE"]), relation("grouping")]),
        Outcome::NotEvaluated(NotEvaluatedReason::IncompleteEvidence)
    );
    for invalid in [vec![], vec![relation("adjacency"), list("classes", &["X"])]] {
        assert_eq!(
            part_of(&invalid),
            Outcome::NotEvaluated(NotEvaluatedReason::InvalidDeclaration)
        );
    }
}

#[test]
fn an_entity_requirement_checks_the_object_class() {
    let services = ServiceRegistry::new();
    let check = |kind: &str, parameters: &[(&str, ParameterValue)]| {
        run(&EntityRequirement, kind, &services, parameters)
    };
    assert_eq!(
        check("IfcWall", &[list("classes", &["IFCWALL"])]),
        Outcome::Meets
    );
    assert_eq!(
        check("IFCWALL", &[list("classes", &["IFCSLAB"])]),
        Outcome::Fails
    );
    assert_eq!(
        check("IFCWALL", &[list("class_patterns", &["IFCW.*"])]),
        Outcome::Meets
    );
    // A predefined type needs the attribute service.
    assert_eq!(
        check(
            "IFCWALL",
            &[
                list("classes", &["IFCWALL"]),
                list("predefined_types", &["SHEAR"])
            ]
        ),
        Outcome::NotEvaluated(NotEvaluatedReason::MissingService)
    );
}
