//! Stable finding identities, and decisions carried over by them.
#![allow(missing_docs)]

use axioval_ir::{
    ChangedFacet, Decision, DecisionError, DecisionStatus, Decisions, Evidence, EvidenceCheck,
    ExternalId, Finding, FindingId, IdentityError, NotEvaluated, NotEvaluatedReason, Object,
    ObjectId, Project, Report, RuleId, Scope, Severity, SourceId, finding_ids,
};
use serde_json::json;

const STABLE: &str = "stable";

fn source(document: &str) -> SourceId {
    SourceId::new("test", document).unwrap()
}

fn id(document: &str, local: u64) -> ObjectId {
    ObjectId::new(source(document), format!("#{local}")).unwrap()
}

/// A wall and a slab with stable aliases and a door without, numbered from
/// `first`: two numberings are two exports of one model.
fn project(document: &str, first: u64) -> Project {
    let alias = |value: &str| ExternalId::new(STABLE, value).unwrap();
    Project::new(vec![
        Object::new(id(document, first), "wall").with_external_id(alias("W")),
        Object::new(id(document, first + 1), "slab").with_external_id(alias("S")),
        Object::new(id(document, first + 2), "door"),
    ])
    .unwrap()
}

/// A wall resting badly on the slab, a door missing a property, and an
/// outcome that could not be evaluated.
fn report(document: &str, first: u64) -> Report {
    let wall = Finding::new(
        RuleId::new("contact").unwrap(),
        id(document, first),
        Severity::Error,
        "wall does not rest on the slab",
    )
    .with_related([id(document, first + 1)])
    .with_evidence([Evidence::exact(source(document), "contact:1")]);
    let door = Finding::new(
        RuleId::new("rating").unwrap(),
        id(document, first + 2),
        Severity::Warning,
        "rating is missing",
    );
    Report {
        findings: vec![wall, door],
        not_evaluated: vec![NotEvaluated {
            rule_id: RuleId::new("headroom").unwrap(),
            scope: Scope::Project,
            reason: NotEvaluatedReason::MissingService,
            message: "no geometry".into(),
            location: None,
        }],
        ..Report::default()
    }
}

fn identified(document: &str, first: u64) -> Report {
    let mut report = report(document, first);
    report
        .identify_findings(&project(document, first), STABLE)
        .unwrap();
    report
}

fn at(date: &str) -> axioval_ir::DateTime {
    date.parse().unwrap()
}

fn decision(finding: &Finding, status: DecisionStatus) -> Decision {
    Decision::new(
        finding.id.unwrap(),
        status,
        "A. Reviewer",
        at("2026-09-27T08:00:00Z"),
    )
    .unwrap()
    .with_basis(finding)
}

#[test]
fn identities_survive_renumbering_where_objects_have_stable_aliases() {
    let (before, after) = (identified("a", 1), identified("a", 100));
    // The wall and slab keep their aliases; the door has none, so its
    // identity follows its local id and does not survive.
    assert_eq!(before.findings[0].id, after.findings[0].id);
    assert_ne!(before.findings[1].id, after.findings[1].id);
    // A revision saved under another file name keeps the aliased identity.
    assert_eq!(before.findings[0].id, identified("b", 1).findings[0].id);
}

#[test]
fn a_key_repeated_across_sources_is_qualified_to_stay_unique() {
    let mut objects: Vec<_> = project("a", 1).objects().cloned().collect();
    objects.extend(project("b", 1).objects().cloned());
    let federated = Project::new(objects).unwrap();
    let mut both = report("a", 1);
    both.findings.extend(report("b", 1).findings);
    let ids = finding_ids(&both, &federated, STABLE).unwrap();
    assert_ne!(ids[0], ids[2]);
    // Alone, the wall's identity is unqualified.
    assert_ne!(ids[0], identified("a", 1).findings[0].id.unwrap());
}

#[test]
fn identifying_a_report_from_another_project_is_refused() {
    let mut report = report("a", 1);
    let error = report
        .identify_findings(&project("b", 1), STABLE)
        .unwrap_err();
    assert!(matches!(error, IdentityError::UnknownObject(id) if id.source.document == "a"));
    assert!(report.findings.iter().all(|finding| finding.id.is_none()));
}

#[test]
fn an_identity_reads_back_from_its_text() {
    let id = identified("a", 1).findings[0].id.unwrap();
    assert_eq!(id.to_string().parse::<FindingId>().unwrap(), id);
    assert!(matches!(
        "not-a-uuid".parse::<FindingId>(),
        Err(IdentityError::NotAnIdentity(_))
    ));
}

#[test]
fn a_report_without_identities_or_decisions_serializes_as_before() {
    let text = serde_json::to_value(report("a", 1)).unwrap();
    let finding = &text["findings"][0];
    assert!(finding.get("id").is_none(), "{finding}");
    assert!(finding.get("decision").is_none(), "{finding}");
    assert!(text.get("stale_decisions").is_none(), "{text}");
}

#[test]
fn a_decided_report_round_trips() {
    let mut report = identified("a", 1);
    let decisions = Decisions::new([
        decision(&report.findings[0], DecisionStatus::Accepted).with_comment("agreed")
    ])
    .unwrap();
    report.apply_decisions(&decisions).unwrap();
    let text = serde_json::to_value(&report).unwrap();
    assert_eq!(
        text["findings"][0]["id"],
        json!(report.findings[0].id.unwrap().to_string())
    );
    assert_eq!(
        text["findings"][0]["decision"],
        json!({"status": "accepted", "author": "A. Reviewer",
               "date": "2026-09-27T08:00:00Z", "comment": "agreed", "evidence": "unchanged"})
    );
    let back: Report = serde_json::from_value(text).unwrap();
    assert_eq!(back, report);
}

#[test]
fn revision_two_carries_revision_ones_decisions_and_lists_stale_ones() {
    let first = identified("a", 1);
    let decisions = Decisions::new([
        decision(&first.findings[0], DecisionStatus::Accepted),
        decision(&first.findings[1], DecisionStatus::Rejected),
    ])
    .unwrap();

    // Revision 2 renumbers everything: the wall's finding is still there
    // under its identity; the door's, keyed by its local id, is not.
    let mut second = identified("a", 100);
    second.apply_decisions(&decisions).unwrap();
    let wall = second.findings[0].decision.as_ref().unwrap();
    assert_eq!(wall.status, DecisionStatus::Accepted);
    assert_eq!(wall.evidence, EvidenceCheck::Unchanged);
    assert!(second.findings[1].decision.is_none());
    assert_eq!(second.stale_decisions().len(), 1);
    assert_eq!(
        second.stale_decisions()[0].finding,
        first.findings[1].id.unwrap()
    );
    // Nothing is hidden: every finding and outcome is still reported.
    assert_eq!(second.findings.len(), 2);
    assert_eq!(second.not_evaluated.len(), 1);
}

#[test]
fn a_changed_severity_or_evidence_is_flagged() {
    let first = identified("a", 1);
    let decisions =
        Decisions::new([decision(&first.findings[0], DecisionStatus::Accepted)]).unwrap();
    let mut second = identified("a", 100);
    second.findings[0].severity = Severity::Warning;
    second.findings[0].evidence.push(Evidence {
        source: source("a"),
        locator: "contact:2".into(),
        exact: false,
    });
    second.apply_decisions(&decisions).unwrap();
    let decision = second.findings[0].decision.as_ref().unwrap();
    assert_eq!(decision.evidence, EvidenceCheck::Changed);
    let changes: Vec<_> = decision
        .changes
        .iter()
        .map(|change| (change.facet, change.decided.as_str(), change.now.as_str()))
        .collect();
    assert_eq!(
        changes,
        [
            (ChangedFacet::Severity, "error", "warning"),
            (ChangedFacet::Evidence, "1", "2"),
            (ChangedFacet::InexactEvidence, "0", "1"),
        ]
    );
}

#[test]
fn a_renumbered_evidence_locator_is_not_a_change() {
    let first = identified("a", 1);
    let decisions =
        Decisions::new([decision(&first.findings[0], DecisionStatus::Accepted)]).unwrap();
    let mut second = identified("a", 100);
    second.findings[0].evidence[0].locator = "contact:57".into();
    second.apply_decisions(&decisions).unwrap();
    assert_eq!(
        second.findings[0].decision.as_ref().unwrap().evidence,
        EvidenceCheck::Unchanged
    );
}

#[test]
fn a_decision_without_a_basis_cannot_tell_whether_anything_changed() {
    let mut report = identified("a", 1);
    let bare = Decision::new(
        report.findings[0].id.unwrap(),
        DecisionStatus::Rejected,
        "A. Reviewer",
        at("2026-09-27T08:00:00Z"),
    )
    .unwrap();
    report
        .apply_decisions(&Decisions::new([bare]).unwrap())
        .unwrap();
    let decision = report.findings[0].decision.as_ref().unwrap();
    assert_eq!(decision.evidence, EvidenceCheck::Unknown);
    assert!(decision.changes.is_empty());
}

#[test]
fn decisions_are_refused_for_a_report_without_identities() {
    let identified = identified("a", 1);
    let decisions =
        Decisions::new([decision(&identified.findings[0], DecisionStatus::Accepted)]).unwrap();
    let mut plain = report("a", 1);
    assert!(matches!(
        plain.apply_decisions(&decisions),
        Err(DecisionError::Unidentified(_))
    ));
    // No decisions need no identities.
    plain.apply_decisions(&Decisions::default()).unwrap();
}

#[test]
fn applying_again_replaces_earlier_decisions() {
    let mut report = identified("a", 1);
    let decisions =
        Decisions::new([decision(&report.findings[0], DecisionStatus::Accepted)]).unwrap();
    report.apply_decisions(&decisions).unwrap();
    report.apply_decisions(&Decisions::default()).unwrap();
    assert!(report.findings[0].decision.is_none());
    assert!(report.stale_decisions().is_empty());
}

#[test]
fn decisions_are_unique_ordered_and_authored() {
    let report = identified("a", 1);
    let wall = decision(&report.findings[0], DecisionStatus::Accepted);
    let door = decision(&report.findings[1], DecisionStatus::Rejected);
    assert_eq!(
        Decisions::new([wall.clone(), wall.clone()]),
        Err(DecisionError::Duplicate(wall.finding))
    );
    let decisions = Decisions::new([door.clone(), wall.clone()]).unwrap();
    let order: Vec<_> = decisions.decisions().iter().map(|d| d.finding).collect();
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted);
    assert!(matches!(
        Decision::new(
            wall.finding,
            DecisionStatus::Open,
            " ",
            at("2026-09-27T08:00:00Z")
        ),
        Err(DecisionError::BlankAuthor(_))
    ));

    // Recording replaces the decision about the same finding.
    let mut decisions = decisions;
    let reopened = Decision {
        status: DecisionStatus::Open,
        ..wall.clone()
    };
    assert_eq!(decisions.record(reopened).unwrap(), Some(wall.clone()));
    assert_eq!(
        decisions.get(wall.finding).unwrap().status,
        DecisionStatus::Open
    );
    assert_eq!(decisions.decisions().len(), 2);
}

#[test]
fn a_decisions_file_is_read_strictly() {
    let report = identified("a", 1);
    let id = report.findings[0].id.unwrap().to_string();
    let file = json!({"decisions": [{
        "finding": id, "status": "rejected", "author": "A. Reviewer",
        "date": "2026-09-27T08:00:00Z", "comment": "false positive"
    }]});
    let decisions: Decisions = serde_json::from_value(file.clone()).unwrap();
    assert_eq!(serde_json::to_value(&decisions).unwrap(), file);

    for bad in [
        json!({"decisions": [], "extra": 1}),
        json!({"decisions": [{"finding": id, "status": "maybe", "author": "A",
                              "date": "2026-09-27T08:00:00Z"}]}),
        json!({"decisions": [{"finding": id, "status": "open", "author": "",
                              "date": "2026-09-27T08:00:00Z"}]}),
        json!({"decisions": [{"finding": id, "status": "open", "author": "A",
                              "date": "2026-09-27T08:00:00"}]}),
        json!({"decisions": [
            {"finding": id, "status": "open", "author": "A", "date": "2026-09-27T08:00:00Z"},
            {"finding": id, "status": "accepted", "author": "B", "date": "2026-09-27T09:00:00Z"}
        ]}),
    ] {
        assert!(
            serde_json::from_value::<Decisions>(bad.clone()).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn assignee_due_date_priority_and_labels_are_carried_to_the_finding() {
    let mut report = identified("a", 1);
    let decided = decision(&report.findings[0], DecisionStatus::Open)
        .with_assignee("C. Engineer")
        .with_due_date(at("2026-10-15T17:00:00+02:00"))
        .with_priority("Critical")
        .with_labels(["structure", "site visit"]);
    let decisions = Decisions::new([decided.clone()]).unwrap();
    let text = serde_json::to_value(&decisions).unwrap();
    assert_eq!(text["decisions"][0]["assigned_to"], "C. Engineer");
    assert_eq!(
        text["decisions"][0]["due_date"],
        "2026-10-15T17:00:00+02:00"
    );
    let back: Decisions = serde_json::from_value(text).unwrap();
    assert_eq!(back, decisions);

    report.apply_decisions(&decisions).unwrap();
    let carried = report.findings[0].decision.as_ref().unwrap();
    assert_eq!(carried.assigned_to.as_deref(), Some("C. Engineer"));
    assert_eq!(carried.due_date, decided.due_date);
    assert_eq!(carried.priority.as_deref(), Some("Critical"));
    assert_eq!(carried.labels, ["structure", "site visit"]);
    let text = serde_json::to_value(&report).unwrap();
    assert_eq!(
        text["findings"][0]["decision"]["labels"],
        json!(["structure", "site visit"])
    );
    let back: Report = serde_json::from_value(text).unwrap();
    assert_eq!(back, report);
}

#[test]
fn decisions_without_review_fields_serialize_byte_identically() {
    let file = r#"{"decisions":[{"finding":"5c1f0c9e-6a0b-5d53-9a8e-2f3b8f6c1d20","status":"rejected","author":"A. Reviewer","date":"2026-09-27T08:00:00Z","comment":"a lining","basis":{"rule_id":"contact","message":"m","severity":"error","evidence":3,"inexact_evidence":0}}]}"#;
    let decisions: Decisions = serde_json::from_str(file).unwrap();
    assert_eq!(serde_json::to_string(&decisions).unwrap(), file);
    let report = r##"{"findings":[{"id":"5c1f0c9e-6a0b-5d53-9a8e-2f3b8f6c1d20","rule_id":"contact","object_id":{"source":{"system":"test","document":"a"},"local_id":"#1"},"severity":"error","message":"m","evidence":[],"decision":{"status":"accepted","author":"A. Reviewer","date":"2026-09-27T08:00:00Z","comment":"agreed","evidence":"unknown"}}],"not_evaluated":[]}"##;
    let parsed: Report = serde_json::from_str(report).unwrap();
    assert_eq!(serde_json::to_string(&parsed).unwrap(), report);
}

#[test]
fn a_blank_assignee_priority_or_label_is_refused() {
    let report = identified("a", 1);
    let base = decision(&report.findings[0], DecisionStatus::Open);
    for (decided, field) in [
        (base.clone().with_assignee(" "), "assigned_to"),
        (base.clone().with_priority(""), "priority"),
        (base.clone().with_labels(["ok", " "]), "labels"),
    ] {
        assert_eq!(
            Decisions::new([decided.clone()]),
            Err(DecisionError::Blank {
                finding: decided.finding,
                field
            })
        );
        assert!(Decisions::default().record(decided).is_err());
    }
}
