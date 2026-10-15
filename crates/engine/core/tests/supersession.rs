//! Objects a federated session evaluates only once, through the object
//! superseding them (#390).
#![allow(missing_docs)]

use axioval_engine::{
    CapabilityEvaluation, CapabilityRegistry, CompiledRule, EvidenceSession, EvidenceSessionError,
    ParameterDescriptor, ParameterType, RuleCapability, RuleContext, Runtime, SourceSnapshot,
    compile,
};
use axioval_ir::{
    DefinitionPackage, Finding, Object, ObjectId, Project, Report, RuleSetPackage, Scope, Severity,
    SourceId, Superseded, SupersessionKind, SupersessionReason,
};

/// Reports every object of the project once, related to every other one,
/// as a clash rule enumerating its counterparts would; and names `b:1`
/// among the related objects of `c:9`, as a host service searching every
/// body it holds might.
struct EveryObject;
impl RuleCapability for EveryObject {
    fn id(&self) -> &'static str {
        "axioval:capability.property-exists"
    }
    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::required(
            "property",
            ParameterType::PropertyReference,
        )]
    }
    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let objects: Vec<&ObjectId> = context.project.objects().map(|o| &o.id).collect();
        let findings = objects
            .iter()
            .map(|subject| {
                let mut related: Vec<ObjectId> = objects
                    .iter()
                    .filter(|other| *other != subject)
                    .map(|other| (*other).clone())
                    .collect();
                if subject.local_id == "9" {
                    related.push(id("b", "1"));
                }
                Finding {
                    id: None,
                    rule_id: rule.id.clone(),
                    scope: Scope::Object((*subject).clone()),
                    severity: Severity::Error,
                    message: "found".into(),
                    related,
                    evidence: Vec::new(),
                    location: None,
                    explanation: None,
                    categories: Vec::new(),
                    decision: None,
                }
            })
            .collect();
        CapabilityEvaluation::evaluated(findings)
    }
}

fn id(document: &str, local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("test", document).unwrap(), local).unwrap()
}

fn member(document: &str, locals: &[&str]) -> EvidenceSession {
    let source = SourceId::new("test", document).unwrap();
    let project = Project::new(
        locals
            .iter()
            .map(|local| Object::new(id(document, local), "wall"))
            .collect(),
    )
    .unwrap();
    EvidenceSession::try_new(
        project,
        [SourceSnapshot::try_new(source, "r1", "sha256:0").unwrap()],
    )
    .unwrap()
}

/// `a` holds the original `a:1`, `b` its copy `b:1` and a wall of its own
/// `b:2`, `c` a duct `c:9` clashing with both.
fn session() -> EvidenceSession {
    EvidenceSession::federate([
        member("a", &["1"]),
        member("b", &["1", "2"]),
        member("c", &["9"]),
    ])
    .unwrap()
}

fn copy() -> Superseded {
    Superseded {
        object: id("b", "1"),
        by: id("a", "1"),
        reason: SupersessionReason::new(SupersessionKind::ById, "same external id and body"),
    }
}

fn run(session: &EvidenceSession) -> Report {
    let definitions: DefinitionPackage = serde_json::from_str(include_str!(
        "../../../../fixtures/schema-v0.1.0/definitions.json"
    ))
    .unwrap();
    let rules: RuleSetPackage = serde_json::from_str(include_str!(
        "../../../../fixtures/schema-v0.1.0/ruleset.json"
    ))
    .unwrap();
    let registry = CapabilityRegistry::new().register(EveryObject).unwrap();
    let plan = compile(&registry, &[definitions], &rules).unwrap();
    Runtime::new(registry).run_session(session, plan).unwrap()
}

fn subjects(report: &Report) -> Vec<String> {
    report
        .findings()
        .iter()
        .map(|finding| finding.scope.object().unwrap().to_string())
        .collect()
}

#[test]
fn a_superseded_object_is_neither_a_subject_nor_a_counterpart() {
    let report = run(&session().with_superseded([copy()]).unwrap());
    assert_eq!(subjects(&report), ["test:a/1", "test:b/2", "test:c/9"]);
    for finding in report.findings() {
        assert!(
            !finding.related.contains(&id("b", "1")),
            "{finding:?} names the copy"
        );
    }
    assert_eq!(report.superseded(), [copy()]);
    // The reason is reported verbatim, its kind a stable code.
    let wire = serde_json::to_value(&report.superseded()[0]).unwrap();
    assert_eq!(
        wire["reason"],
        serde_json::json!({"kind": "by-id", "text": "same external id and body"})
    );
}

#[test]
fn an_object_a_host_service_still_names_is_named_by_its_superseding_object_once() {
    let report = run(&session().with_superseded([copy()]).unwrap());
    let duct = &report.findings()[2];
    assert_eq!(duct.related, [id("a", "1"), id("b", "2")]);
}

#[test]
fn without_a_declaration_the_report_is_unchanged() {
    let report = run(&session());
    assert_eq!(
        subjects(&report),
        ["test:a/1", "test:b/1", "test:b/2", "test:c/9"]
    );
    assert!(report.superseded().is_empty());
    let wire = serde_json::to_value(&report).unwrap();
    assert!(wire.get("superseded").is_none());
    let empty = run(&session().with_superseded([]).unwrap());
    assert_eq!(
        serde_json::to_string(&empty).unwrap(),
        serde_json::to_string(&report).unwrap()
    );
}

#[test]
fn a_declaration_fails_closed() {
    let refused = |entries: Vec<Superseded>| session().with_superseded(entries).err().unwrap();
    let entry = |object: ObjectId, by: ObjectId| Superseded {
        object,
        by,
        reason: SupersessionReason::new(
            SupersessionKind::Declared,
            "a reduced copy inside its original, as the viewer matched it",
        ),
    };
    assert_eq!(
        refused(vec![entry(id("b", "7"), id("a", "1"))]),
        EvidenceSessionError::UnknownObject(id("b", "7"))
    );
    assert_eq!(
        refused(vec![entry(id("x", "1"), id("a", "1"))]),
        EvidenceSessionError::UnknownObject(id("x", "1"))
    );
    assert_eq!(
        refused(vec![entry(id("b", "1"), id("b", "1"))]),
        EvidenceSessionError::SelfSupersession(id("b", "1"))
    );
    assert_eq!(
        refused(vec![entry(id("b", "1"), id("b", "2"))]),
        EvidenceSessionError::SameSourceSupersession(id("b", "1"))
    );
    assert_eq!(
        refused(vec![
            entry(id("b", "1"), id("a", "1")),
            entry(id("b", "1"), id("c", "9")),
        ]),
        EvidenceSessionError::SupersededTwice(id("b", "1"))
    );
    // A chain, and a cycle, are refused rather than resolved.
    assert_eq!(
        refused(vec![
            entry(id("b", "1"), id("a", "1")),
            entry(id("a", "1"), id("c", "9")),
        ]),
        EvidenceSessionError::SupersessionChain(id("a", "1"))
    );
    assert_eq!(
        refused(vec![
            entry(id("b", "1"), id("a", "1")),
            entry(id("a", "1"), id("b", "1")),
        ]),
        EvidenceSessionError::SupersessionChain(id("b", "1"))
    );
    assert_eq!(
        refused(vec![Superseded {
            reason: SupersessionReason::new(SupersessionKind::Declared, " "),
            ..copy()
        }]),
        EvidenceSessionError::BlankSupersessionReason(id("b", "1"))
    );
    // Declaring the same entry twice is a no-op.
    let twice = session().with_superseded([copy(), copy()]).unwrap();
    assert_eq!(twice.superseded().cloned().collect::<Vec<_>>(), [copy()]);
}
