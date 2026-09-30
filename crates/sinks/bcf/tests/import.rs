//! BCF archives read back into review decisions.
#![allow(missing_docs, clippy::doc_markdown)]

use std::io::{Cursor, Write};

use axioval_bcf::{
    IFC_GLOBAL_ID_SCHEME, ImportError, MAX_ATTRIBUTES_PER_TAG, Options, Unmatched, export, import,
    import_topics,
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

    let imported = import(&bytes, &report, &model()).unwrap();
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
    assert_eq!(commented.comment, "the rating is in the door schedule");
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
            .comment,
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
    let imported = import(&bytes, &report, &model()).unwrap();
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
    let imported = import(&bytes, &report, &model()).unwrap();
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
    let imported = import(&bytes, &fixed, &model()).unwrap();
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
    let imported = import_topics(&markups, &report, &model()).unwrap();
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
    let result = import_topics([&markup, &markup], &report, &model());
    assert!(
        matches!(result, Err(ImportError::Decisions(_))),
        "{result:?}"
    );
}

#[test]
fn a_tag_with_too_many_attributes_is_refused_before_parsing() {
    let mut attributes = String::new();
    for n in 0..=MAX_ATTRIBUTES_PER_TAG {
        std::fmt::Write::write_fmt(&mut attributes, format_args!(" a{n}=\"{n}\"")).unwrap();
    }
    let mut bytes = Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut bytes);
    zip.start_file(
        "3f2504e0-4f89-41d3-9a0c-0305e82c3301/markup.bcf",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    write!(zip, "<Markup{attributes}/>").unwrap();
    zip.finish().unwrap();
    let result = import(bytes.get_ref(), &report(), &model());
    assert!(
        matches!(result, Err(ImportError::TooManyAttributes { .. })),
        "{result:?}"
    );
}

#[test]
fn bytes_that_are_no_archive_are_refused() {
    let result = import(b"not a zip", &report(), &model());
    assert!(matches!(result, Err(ImportError::Archive(_))), "{result:?}");
}
