//! `axioval:capability.population`: how many objects a rule selects.
#![allow(missing_docs)]

use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, RuleCapability, RuleContext, ServiceRegistry,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::{NotEvaluatedReason, Object, ObjectId, Project, RuleId, SourceId};
use axioval_rules::PopulationRequirement;

fn evaluate(objects: usize, selector: Selector, bounds: &[(&str, i64)]) -> CapabilityEvaluation {
    let source = SourceId::new("cad", "native-model").unwrap();
    let project = Project::new(
        (0..objects)
            .map(|index| {
                Object::new(
                    ObjectId::new(source.clone(), format!("wall-{index}")).unwrap(),
                    "wall",
                )
            })
            .collect(),
    )
    .unwrap();
    let rule = CompiledRule {
        id: RuleId::new("population").unwrap(),
        capability: "axioval:capability.population".into(),
        severity: Severity::Warning,
        selector,
        parameters: bounds
            .iter()
            .map(|(name, value)| {
                (
                    (*name).to_owned(),
                    ParameterValue::Integer { value: *value },
                )
            })
            .collect::<BTreeMap<_, _>>(),
    };
    PopulationRequirement.evaluate(
        &RuleContext {
            project: &project,
            services: &ServiceRegistry::new(),
        },
        &rule,
    )
}

/// A selector whose membership cannot be decided without a service.
fn undecided() -> Selector {
    Selector::Classification {
        system: "Uniclass".into(),
        code: "EF".into(),
        include_descendants: false,
    }
}

#[test]
fn too_few_is_one_finding_about_the_rule() {
    let evaluation = evaluate(0, Selector::All, &[("min", 1)]);
    assert!(evaluation.findings().is_empty());
    assert_eq!(evaluation.rule_findings().len(), 1);
    let finding = &evaluation.rule_findings()[0];
    assert!(finding.related.is_empty());
    assert_eq!(finding.severity, axioval_ir::Severity::Warning);
    assert!(
        evaluate(1, Selector::All, &[("min", 1)])
            .rule_findings()
            .is_empty()
    );
}

#[test]
fn too_many_names_the_objects_and_none_allowed_flags_each() {
    let evaluation = evaluate(3, Selector::All, &[("max", 2)]);
    assert_eq!(evaluation.rule_findings().len(), 1);
    assert_eq!(evaluation.rule_findings()[0].related.len(), 3);
    let none = evaluate(2, Selector::All, &[("max", 0)]);
    assert!(none.rule_findings().is_empty());
    assert_eq!(none.findings().len(), 2);
    assert!(
        evaluate(0, Selector::All, &[("max", 0)])
            .findings()
            .is_empty()
    );
}

#[test]
fn undecided_membership_that_could_flip_the_verdict_is_not_evaluated() {
    let evaluation = evaluate(2, undecided(), &[("min", 1)]);
    assert!(evaluation.rule_findings().is_empty());
    // Two objects undecided by selection, and the rule itself.
    assert!(
        evaluation
            .not_evaluated_outcomes()
            .iter()
            .any(|outcome| outcome.object_id().is_none()
                && *outcome.reason() == NotEvaluatedReason::IncompleteEvidence)
    );
}

#[test]
fn bounds_are_validated() {
    for bounds in [vec![], vec![("min", -1)], vec![("min", 3), ("max", 2)]] {
        let evaluation = evaluate(1, Selector::All, &bounds);
        assert_eq!(evaluation.not_evaluated_outcomes().len(), 1, "{bounds:?}");
        assert_eq!(
            evaluation.not_evaluated_outcomes()[0].reason(),
            &NotEvaluatedReason::InvalidDeclaration
        );
    }
}
