//! BCF archives read back into review decisions.
#![allow(missing_docs, clippy::doc_markdown)]

use std::collections::BTreeMap;

use axioval_bcf::{
    IFC_GLOBAL_ID_SCHEME, ImportError, Options, Unmatched, export, import, import_topics,
};
use axioval_ir::{
    Decision, DecisionStatus, Decisions, Evidence, ExternalId, Finding, NotEvaluated,
    NotEvaluatedReason, Object, ObjectId, Project, Report, RuleId, Scope, Severity, SourceId,
};
use openbim_bcf::write::{self, Comment};

const WALL: &str = "2O2Fr$t4X7Zf8NOew3FLOH";
const SLAB: &str = "0000000000000000000001";

fn id(local: u64) -> ObjectId {
    ObjectId::new(
        SourceId::new("ifc-step", "a.ifc").unwrap(),
        format!("#{local}"),
    )
    .unwrap()
}

/// A wall and a slab with GlobalId aliases, and a door without one.
fn model() -> Project {
    let alias = |value: &str| ExternalId::new(IFC_GLOBAL_ID_SCHEME, value).unwrap();
    Project::new(vec![
        Object::new(id(1), "IFCWALL").with_external_id(alias(WALL)),
        Object::new(id(2), "IFCSLAB").with_external_id(alias(SLAB)),
        Object::new(id(3), "IFCDOOR"),
    ])
    .unwrap()
}

/// A wall finding, a door finding and a rule that could not run, with
/// identities.
fn report() -> Report {
    let source = SourceId::new("ifc-step", "a.ifc").unwrap();
    let finding = |rule: &str, object, severity, message: &str| Finding {
        explanation: None,
        id: None,
        decision: None,
        rule_id: RuleId::new(rule).unwrap(),
        scope: Scope::Object(id(object)),
        severity,
        message: message.into(),
        related: vec![],
        evidence: vec![Evidence::exact(source.clone(), "e")],
        location: None,
        categories: Vec::new(),
    };
    let mut report = Report {
        findings: vec![
            finding(
                "slab-contact",
                1,
                Severity::Error,
                "Wall has insufficient contact with the slab below",
            )
            .with_related([id(2)]),
            finding(
                "door-fire-rating",
                3,
                Severity::Warning,
                "FireRating is missing",
            ),
        ],
        not_evaluated: vec![NotEvaluated {
            explanation: None,
            rule_id: RuleId::new("stair-headroom").unwrap(),
            scope: Scope::Project,
            reason: NotEvaluatedReason::MissingService,
            message: "no geometry service is registered".into(),
            location: None,
        }],
        ..Report::default()
    };
    report
        .identify_findings(&model(), IFC_GLOBAL_ID_SCHEME)
        .unwrap();
    report
}

fn options() -> Options {
    Options::new("axioval-check", "2026-09-26T10:00:00Z")
}

#[test]
fn a_topic_closed_elsewhere_is_accepted_and_a_comment_made_elsewhere_is_carried() {
    let report = report();
    let mut exported = export(&report, &model(), &options()).unwrap().document;
    // Another BCF tool closes the wall's topic and comments on the door's.
    exported.topics[0].topic_status = Some("Closed".into());
    exported.topics[1].comments.push(Comment {
        guid: "7e1d2c3b-4a59-4687-9a1b-2c3d4e5f6a7b".into(),
        date: "2026-09-28T09:30:00+02:00".into(),
        author: "B. Reviewer".into(),
        comment: "the rating is in the door schedule".into(),
        viewpoint: None,
    });
    let bytes = write::to_vec(&exported).unwrap();

    let imported = import(&bytes, &report, &model(), &BTreeMap::new()).unwrap();
    let wall = report.findings[0].id.unwrap();
    let door = report.findings[1].id.unwrap();
    let decided = imported.decisions.get(wall).unwrap();
    assert_eq!(decided.status, DecisionStatus::Accepted);
    // No modification was recorded, so the topic's creation stands in.
    assert_eq!(decided.author, "axioval-check");
    assert_eq!(decided.date.to_string(), "2026-09-26T10:00:00Z");
    let commented = imported.decisions.get(door).unwrap();
    assert_eq!(commented.status, DecisionStatus::Open);
    assert_eq!(commented.author, "B. Reviewer");
    assert_eq!(commented.date.to_string(), "2026-09-28T09:30:00+02:00");
    assert_eq!(commented.comments.len(), 1);
    assert_eq!(
        commented.comments[0].text,
        "the rating is in the door schedule"
    );
    assert_eq!(commented.comments[0].author, "B. Reviewer");
    assert_eq!(imported.decisions.decisions().len(), 2);

    // The not-evaluated outcome's topic is listed, never decided.
    assert_eq!(imported.unmatched.len(), 1);
    assert_eq!(imported.unmatched[0].reason, Unmatched::NotEvaluated);
    assert_eq!(
        imported.unmatched[0].title.as_deref(),
        Some("no geometry service is registered")
    );

    // Applied, the decisions mark the findings and nothing is stale.
    let mut decided_report = report.clone();
    decided_report.apply_decisions(&imported.decisions).unwrap();
    assert_eq!(
        decided_report.findings[0].decision.as_ref().unwrap().status,
        DecisionStatus::Accepted
    );
    assert_eq!(
        decided_report.findings[1]
            .decision
            .as_ref()
            .unwrap()
            .comments[0]
            .text,
        "the rating is in the door schedule"
    );
    assert!(decided_report.stale_decisions.is_empty());
}

#[test]
fn an_untouched_export_decides_nothing() {
    let report = report();
    let bytes = export(&report, &model(), &options())
        .unwrap()
        .to_bytes()
        .unwrap();
    let imported = import(&bytes, &report, &model(), &BTreeMap::new()).unwrap();
    assert!(imported.decisions.is_empty());
    assert_eq!(imported.unmatched.len(), 1);
}

#[test]
fn exported_decisions_read_back_unchanged() {
    let mut report = report();
    let date = "2026-09-27T08:00:00+02:00".parse().unwrap();
    let wall = report.findings[0].id.unwrap();
    let door = report.findings[1].id.unwrap();
    let decisions = Decisions::new([
        Decision::new(wall, DecisionStatus::Accepted, "A. Reviewer", date)
            .unwrap()
            .with_comment("agreed with the structural engineer"),
        Decision::new(door, DecisionStatus::Rejected, "B. Reviewer", date).unwrap(),
    ])
    .unwrap();
    // With a basis, so the export also writes the change note.
    let mut recorded = decisions.clone();
    let changed = Decision {
        basis: Some(axioval_ir::DecisionBasis::of(&report.findings[0])),
        ..decisions.get(wall).unwrap().clone()
    };
    recorded.record(changed).unwrap();
    report.findings[0].severity = Severity::Warning;
    report.apply_decisions(&recorded).unwrap();
    let bytes = export(&report, &model(), &options())
        .unwrap()
        .to_bytes()
        .unwrap();
    let imported = import(&bytes, &report, &model(), &BTreeMap::new()).unwrap();
    assert_eq!(imported.decisions, decisions);
}

#[test]
fn topics_of_other_tools_and_of_fixed_findings_are_listed_not_dropped() {
    let report = report();
    let mut exported = export(&report, &model(), &options()).unwrap().document;
    // A topic another tool made, never a finding of this report.
    let mut foreign = exported.topics[1].clone();
    foreign.guid = "3f2504e0-4f89-41d3-9a0c-0305e82c3301".into();
    foreign.title = "Duct clashes with beam".into();
    foreign.topic_status = Some("Closed".into());
    foreign.viewpoints.clear();
    exported.topics.push(foreign);
    let bytes = write::to_vec(&exported).unwrap();

    // The door was fixed: the next report has only the wall.
    let mut fixed = report.clone();
    fixed.findings.truncate(1);
    let imported = import(&bytes, &fixed, &model(), &BTreeMap::new()).unwrap();
    assert!(imported.decisions.is_empty());
    let listed: Vec<_> = imported
        .unmatched
        .iter()
        .map(|topic| (topic.title.as_deref().unwrap(), topic.reason.clone()))
        .collect();
    assert_eq!(
        listed,
        [
            ("FireRating is missing", Unmatched::NoFinding),
            ("no geometry service is registered", Unmatched::NotEvaluated),
            ("Duct clashes with beam", Unmatched::NoFinding),
        ]
    );
    assert_eq!(imported.unmatched[2].status.as_deref(), Some("Closed"));
}

#[test]
fn a_topic_without_a_guid_or_with_an_unreadable_date_is_listed() {
    let report = report();
    let bytes = export(&report, &model(), &options())
        .unwrap()
        .to_bytes()
        .unwrap();
    let archive = openbim_bcf::read_slice(&bytes).unwrap();
    let mut markups: Vec<_> = archive.topics().cloned().collect();
    // A comment dated without a UTC offset cannot be placed in time.
    markups[0].comments.push(openbim_bcf::Comment {
        guid: Some("7e1d2c3b-4a59-4687-9a1b-2c3d4e5f6a7b".into()),
        date: Some("2026-09-28T09:30:00".into()),
        author: Some("B. Reviewer".into()),
        comment: Some("looked at it".into()),
        viewpoint: None,
        modified_date: None,
        modified_author: None,
    });
    markups[1].topic.guid = Some("not-a-guid".into());
    let imported = import_topics(&markups, &report, &model(), &BTreeMap::new()).unwrap();
    assert!(imported.decisions.is_empty());
    assert!(
        matches!(&imported.unmatched[0].reason, Unmatched::Unreadable(why) if why.contains("UTC offset")),
        "{:?}",
        imported.unmatched
    );
    assert_eq!(imported.unmatched[1].reason, Unmatched::NoGuid);
}

#[test]
fn two_topics_about_one_finding_are_refused() {
    let report = report();
    let mut exported = export(&report, &model(), &options()).unwrap().document;
    exported.topics[0].topic_status = Some("Closed".into());
    let archive = openbim_bcf::read_slice(&write::to_vec(&exported).unwrap()).unwrap();
    let markup = archive.topics().next().unwrap().clone();
    let result = import_topics([&markup, &markup], &report, &model(), &BTreeMap::new());
    assert!(
        matches!(result, Err(ImportError::Decisions(_))),
        "{result:?}"
    );
}

#[test]
fn bytes_that_are_no_archive_are_refused() {
    let result = import(b"not a zip", &report(), &model(), &BTreeMap::new());
    assert!(matches!(result, Err(ImportError::Read(_))), "{result:?}");
}

/// Assignee, due date, priority and labels of a review.
mod review {
    use super::{BTreeMap, model, options, report};
    use axioval_bcf::{Bounds, Options, Unmatched, Version, export, import, import_topics};
    use axioval_ir::{Decision, DecisionStatus, Decisions};

    fn reviewed() -> (axioval_ir::Report, Decisions) {
        let mut report = report();
        let wall = report.findings[0].id.unwrap();
        let decisions = Decisions::new([Decision::new(
            wall,
            DecisionStatus::Open,
            "A. Reviewer",
            "2026-09-27T08:00:00Z".parse().unwrap(),
        )
        .unwrap()
        .with_comment("needs a site visit")
        .with_assignee("C. Engineer")
        .with_due_date("2026-10-15T17:00:00+02:00".parse().unwrap())
        .with_priority("Critical")
        .with_labels(["structure", "site visit"])])
        .unwrap();
        report.apply_decisions(&decisions).unwrap();
        (report, decisions)
    }

    #[test]
    fn priority_and_labels_are_written_and_read_back_unchanged() {
        let (report, decisions) = reviewed();
        let export = export(&report, &model(), &options()).unwrap();
        let wall = &export.document.topics[0];
        assert_eq!(wall.priority.as_deref(), Some("Critical"));
        assert_eq!(wall.labels, ["slab-contact", "structure", "site visit"]);
        // The door keeps the priority of its severity.
        assert_eq!(
            export.document.topics[1].priority.as_deref(),
            Some("Normal")
        );

        let imported = import(
            &export.to_bytes().unwrap(),
            &report,
            &model(),
            &BTreeMap::new(),
        )
        .unwrap();
        let expected = decisions.decisions()[0].clone();
        let back = imported.decisions.get(expected.finding).unwrap();
        assert_eq!(back.priority, expected.priority);
        assert_eq!(back.labels, expected.labels);
        assert_eq!(back.comments, expected.comments);
        assert_eq!(
            (&back.author, back.date, back.status),
            (&expected.author, expected.date, expected.status)
        );
    }

    #[test]
    fn a_decision_with_an_assignee_and_due_date_round_trips_unchanged() {
        let (report, decisions) = reviewed();
        for version in [Version::V2_1, Version::V3_0] {
            let options = Options {
                version,
                bounds: Some(bounds()),
                ..options()
            };
            let export = export(&report, &model(), &options).unwrap();
            let wall = &export.document.topics[0];
            assert_eq!(wall.assigned_to.as_deref(), Some("C. Engineer"));
            assert_eq!(wall.due_date.as_deref(), Some("2026-10-15T17:00:00+02:00"));
            let bytes = export.to_bytes().unwrap();
            let archive = openbim_bcf::read_slice(&bytes).unwrap();
            assert!(
                archive.diagnostics().is_empty(),
                "{:?}",
                archive.diagnostics()
            );
            let imported = import(&bytes, &report, &model(), &BTreeMap::new()).unwrap();
            assert_eq!(imported.decisions, decisions, "{version:?}");
        }
        // Without them the topic carries neither.
        let plain = export_of(&super::report());
        assert!(
            plain
                .document
                .topics
                .iter()
                .all(|t| t.assigned_to.is_none() && t.due_date.is_none())
        );
    }

    /// Bounds for every object, so BCF 3.0 gets its cameras.
    fn bounds() -> BTreeMap<axioval_ir::ObjectId, Bounds> {
        (1_u32..=3)
            .map(|local| {
                let at = f64::from(local);
                (
                    super::id(u64::from(local)),
                    Bounds::new([at, 0.0, 0.0], [at + 1.0, 1.0, 1.0]).unwrap(),
                )
            })
            .collect()
    }

    fn export_of(report: &axioval_ir::Report) -> axioval_bcf::Export {
        export(report, &model(), &options()).unwrap()
    }

    #[test]
    fn assignee_due_date_and_reviewer_labels_are_read_from_their_topic_fields() {
        let report = report();
        let bytes = export_of(&report).to_bytes().unwrap();
        let archive = openbim_bcf::read_slice(&bytes).unwrap();
        let mut markups: Vec<_> = archive.topics().cloned().collect();
        // Another tool assigns the untouched wall topic, sets a due date and
        // labels it beside the export's own labels.
        let topic = &mut markups[0].topic;
        topic.assigned_to = Some("C. Engineer".into());
        topic.due_date = Some("2026-10-15T17:00:00+02:00".into());
        topic.labels = vec![
            "slab-contact".into(),
            "Folder: Structure".into(),
            "structural".into(),
            "Storey: Level 1".into(),
            "site visit".into(),
            "Decision changed".into(),
        ];
        let rule_labels = BTreeMap::from([("slab-contact".to_owned(), vec!["structural".into()])]);
        let imported = import_topics(&markups, &report, &model(), &rule_labels).unwrap();
        let wall = imported
            .decisions
            .get(report.findings[0].id.unwrap())
            .unwrap();
        assert_eq!(wall.status, DecisionStatus::Open);
        assert_eq!(wall.assigned_to.as_deref(), Some("C. Engineer"));
        assert_eq!(
            wall.due_date.unwrap().to_string(),
            "2026-10-15T17:00:00+02:00"
        );
        assert_eq!(wall.labels, ["site visit"]);
        assert_eq!(wall.priority, None);
        assert_eq!(wall.author, "axioval-check");

        // A due date without a UTC offset cannot be placed in time.
        markups[0].topic.due_date = Some("2026-10-15".into());
        let imported = import_topics(&markups, &report, &model(), &rule_labels).unwrap();
        assert!(imported.decisions.is_empty());
        assert!(
            matches!(&imported.unmatched[0].reason, Unmatched::Unreadable(why) if why.contains("due date")),
            "{:?}",
            imported.unmatched
        );
    }
}

/// Comment threads written as BCF comments and read back.
mod threads {
    use super::{BTreeMap, model, options, report};
    use axioval_bcf::{export, import};
    use axioval_ir::{Decision, DecisionComment, DecisionStatus, Decisions};

    fn at(date: &str) -> axioval_ir::DateTime {
        date.parse().unwrap()
    }

    /// Exports `decisions` applied to the report and imports the archive.
    fn round_trip(decisions: &Decisions) -> (Decisions, Vec<openbim_bcf::Markup>) {
        let mut report = report();
        report.apply_decisions(decisions).unwrap();
        let bytes = export(&report, &model(), &options())
            .unwrap()
            .to_bytes()
            .unwrap();
        let imported = import(&bytes, &report, &model(), &BTreeMap::new()).unwrap();
        let archive = openbim_bcf::read_slice(&bytes).unwrap();
        assert!(
            archive.diagnostics().is_empty(),
            "{:?}",
            archive.diagnostics()
        );
        (imported.decisions, archive.topics().cloned().collect())
    }

    #[test]
    fn a_three_comment_thread_round_trips() {
        let wall = report().findings[0].id.unwrap();
        let decisions = Decisions::new([Decision::new(
            wall,
            DecisionStatus::Rejected,
            "A. Reviewer",
            at("2026-09-27T08:00:00Z"),
        )
        .unwrap()
        .with_comment("the wall is a lining")
        .with_reply(DecisionComment::new(
            "B. Engineer",
            at("2026-09-28T09:30:00+02:00"),
            "agreed, no contact needed",
        ))
        .with_reply(
            DecisionComment::new("C. Site", at("2026-09-29T08:00:00Z"), "confirmed on site")
                .with_id("7e1d2c3b-4a59-4687-9a1b-2c3d4e5f6a7b".parse().unwrap()),
        )])
        .unwrap();
        let (back, markups) = round_trip(&decisions);
        assert_eq!(back, decisions);

        // Three BCF comments: the decision's own, then the replies in order.
        let wall_topic = &markups[0];
        let texts: Vec<_> = wall_topic
            .comments
            .iter()
            .map(|c| (c.author.as_deref().unwrap(), c.comment.as_deref().unwrap()))
            .collect();
        assert_eq!(
            texts,
            [
                ("A. Reviewer", "Rejected: the wall is a lining"),
                ("B. Engineer", "agreed, no contact needed"),
                ("C. Site", "confirmed on site"),
            ]
        );
        assert_eq!(
            wall_topic.comments[2].guid.as_deref(),
            Some("7e1d2c3b-4a59-4687-9a1b-2c3d4e5f6a7b")
        );
        // Re-exporting reproduces every comment GUID.
        let (_, again) = round_trip(&back);
        assert_eq!(again[0].comments, wall_topic.comments);
    }

    #[test]
    fn a_thread_not_opened_by_the_decider_keeps_its_first_comment_apart() {
        let wall = report().findings[0].id.unwrap();
        let same = DecisionComment::new("B. Engineer", at("2026-09-28T08:00:00Z"), "+1");
        let decisions = Decisions::new([Decision::new(
            wall,
            DecisionStatus::Accepted,
            "A. Reviewer",
            at("2026-09-27T08:00:00Z"),
        )
        .unwrap()
        .with_reply(same.clone())
        // The same words twice keep two distinct GUIDs.
        .with_reply(same)])
        .unwrap();
        let (back, markups) = round_trip(&decisions);
        assert_eq!(back, decisions);
        assert_eq!(markups[0].comments.len(), 3);
        assert_eq!(markups[0].comments[0].comment.as_deref(), Some("Accepted"));
        assert_ne!(markups[0].comments[1].guid, markups[0].comments[2].guid);
    }
}
