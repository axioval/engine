//! Reports exported as BCF 2.1 and read back.
#![allow(missing_docs, clippy::doc_markdown)]

use std::io::{Cursor, Read};

use axioval_bcf::{ExportError, IFC_GLOBAL_ID_SCHEME, NOT_EVALUATED_TOPIC_TYPE, Options, export};
use axioval_ir::{
    Evidence, ExternalId, Finding, NotEvaluated, NotEvaluatedReason, Object, ObjectId, Project,
    Report, RuleId, Scope, Severity, SourceId,
};

const WALL: &str = "2O2Fr$t4X7Zf8NOew3FLOH";
const SLAB: &str = "0000000000000000000001";

/// A wall and a slab with GlobalId aliases, and a door without one, as the
/// IFC adapter maps them. `first` numbers the objects, so two numberings model
/// two exports of one model.
///
/// Built by hand rather than imported: a dev-dependency on the unpublished
/// `axioval-ifc` breaks workspace package verification. The facade's
/// `ifc_bcf` test runs the same path through the real adapter.
fn model(document: &str, first: u64) -> Project {
    let alias = |value: &str| ExternalId::new(IFC_GLOBAL_ID_SCHEME, value).unwrap();
    Project::new(vec![
        Object::new(id(document, first), "IFCWALL").with_external_id(alias(WALL)),
        Object::new(id(document, first + 1), "IFCSLAB").with_external_id(alias(SLAB)),
        Object::new(id(document, first + 2), "IFCDOOR"),
    ])
    .unwrap()
}

fn id(document: &str, local: u64) -> ObjectId {
    ObjectId::new(
        SourceId::new("ifc-step", document).unwrap(),
        format!("#{local}"),
    )
    .unwrap()
}

/// A wall that fails to rest on the slab, a door (no alias) missing a
/// property, and a rule that could not run at all.
fn report(document: &str, first: u64) -> Report {
    let source = SourceId::new("ifc-step", document).unwrap();
    Report {
        findings: vec![
            Finding {
                rule_id: RuleId::new("slab-contact").unwrap(),
                scope: Scope::Object(id(document, first)),
                severity: Severity::Error,
                message: "Wall has insufficient contact with the slab below".into(),
                related: vec![],
                evidence: vec![Evidence::exact(source, "contact:wall")],
            }
            .with_related([id(document, first + 1)]),
            Finding {
                rule_id: RuleId::new("door-fire-rating").unwrap(),
                scope: Scope::Object(id(document, first + 2)),
                severity: Severity::Warning,
                message: "FireRating is missing".into(),
                related: vec![],
                evidence: vec![],
            },
        ],
        not_evaluated: vec![NotEvaluated {
            rule_id: RuleId::new("stair-headroom").unwrap(),
            scope: Scope::Project,
            reason: NotEvaluatedReason::MissingService,
            message: "no geometry service is registered".into(),
        }],
    }
}

fn options() -> Options {
    Options::new("axioval-check", "2026-09-26T10:00:00Z")
}

/// Text of every viewpoint file in the archive.
fn viewpoints(bytes: &[u8]) -> Vec<String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut texts = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        let is_view = std::path::Path::new(entry.name())
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("bcfv"));
        if is_view {
            let mut text = String::new();
            entry.read_to_string(&mut text).unwrap();
            texts.push(text);
        }
    }
    texts
}

#[test]
fn every_report_entry_becomes_a_topic_that_reads_back_cleanly() {
    let export = export(&report("a.ifc", 1), &model("a.ifc", 1), &options()).unwrap();
    let bytes = export.to_bytes().unwrap();
    let archive = openbim_bcf::read_slice(&bytes).unwrap();
    assert!(
        archive.diagnostics().is_empty(),
        "{:?}",
        archive.diagnostics()
    );

    let topics: Vec<_> = archive.topics().map(|markup| &markup.topic).collect();
    assert_eq!(topics.len(), 3);
    let by_title = |title: &str| {
        *topics
            .iter()
            .find(|topic| topic.title.as_deref() == Some(title))
            .unwrap_or_else(|| panic!("no topic titled {title:?}"))
    };
    let wall = by_title("Wall has insufficient contact with the slab below");
    assert_eq!(wall.topic_type.as_deref(), Some("Error"));
    assert_eq!(wall.topic_status.as_deref(), Some("Open"));
    assert_eq!(wall.labels, ["slab-contact"]);
    let description = wall.description.as_deref().unwrap();
    assert!(
        description.contains("Evidence (exact): contact:wall"),
        "{description}"
    );

    let skipped = by_title("no geometry service is registered");
    assert_eq!(
        skipped.topic_type.as_deref(),
        Some(NOT_EVALUATED_TOPIC_TYPE)
    );
    assert!(
        skipped
            .description
            .as_deref()
            .unwrap()
            .contains("missing service")
    );

    // Only the wall's topic can select anything: subject first, then the slab.
    let views = viewpoints(&bytes);
    assert_eq!(views.len(), 1, "{views:?}");
    let (wall_at, slab_at) = (views[0].find(WALL).unwrap(), views[0].find(SLAB).unwrap());
    assert!(wall_at < slab_at, "{}", views[0]);

    // The door's topic is written, and the gap is reported, not hidden.
    assert!(
        by_title("FireRating is missing")
            .description
            .as_deref()
            .unwrap()
            .contains("#3")
    );
    assert_eq!(export.unanchored, [id("a.ifc", 3)]);
}

#[test]
fn topic_guids_survive_renumbering_on_re_export() {
    // Same GlobalIds, different STEP numbers: a re-export of the same model.
    let guids = |first: u64| -> Vec<String> {
        export(&report("a.ifc", first), &model("a.ifc", first), &options())
            .unwrap()
            .document
            .topics
            .into_iter()
            .map(|topic| topic.guid)
            .collect()
    };
    let (before, after) = (guids(1), guids(100));
    // The wall's issue keeps its GUID; the door has no GlobalId, so its
    // identity is the STEP number and it cannot be tracked across exports.
    assert_eq!(before[0], after[0]);
    assert_ne!(before[1], after[1]);
    assert_eq!(before[2], after[2]);
}

#[test]
fn identical_input_writes_identical_bytes() {
    let bytes = || {
        export(&report("a.ifc", 1), &model("a.ifc", 1), &options())
            .unwrap()
            .to_bytes()
            .unwrap()
    };
    assert_eq!(bytes(), bytes());
}

#[test]
fn a_federation_of_two_revisions_keeps_every_guid_unique() {
    let mut objects: Vec<_> = model("a.ifc", 1).objects().cloned().collect();
    objects.extend(model("b.ifc", 1).objects().cloned());
    let project = Project::new(objects).unwrap();
    let mut both = report("a.ifc", 1);
    both.findings.extend(report("b.ifc", 1).findings);
    let export = export(&both, &project, &options()).unwrap();
    // The writer refuses repeated GUIDs, so writing proves they are unique.
    let archive = openbim_bcf::read_slice(&export.to_bytes().unwrap()).unwrap();
    assert_eq!(archive.topic_count(), 5);
}

#[test]
fn not_evaluated_outcomes_can_be_left_out() {
    let options = Options {
        include_not_evaluated: false,
        ..options()
    };
    let export = export(&report("a.ifc", 1), &model("a.ifc", 1), &options).unwrap();
    assert_eq!(export.document.topics.len(), 2);
}

#[test]
fn a_report_from_another_project_is_refused() {
    let error = export(&report("a.ifc", 1), &model("b.ifc", 1), &options()).unwrap_err();
    assert!(matches!(error, ExportError::UnknownObject(id) if id.source.document == "a.ifc"));
}

#[test]
fn an_invalid_date_is_refused_by_the_writer() {
    let options = Options::new("axioval-check", "yesterday");
    let export = export(&report("a.ifc", 1), &model("a.ifc", 1), &options).unwrap();
    assert!(matches!(export.to_bytes(), Err(ExportError::Write(_))));
}

#[test]
fn related_objects_alone_are_never_selected() {
    // The door has no GlobalId; showing only the wall would point the
    // reviewer at the wrong element.
    let mut report = report("a.ifc", 1);
    report.findings = vec![
        Finding {
            rule_id: RuleId::new("door-in-wall").unwrap(),
            scope: Scope::Object(id("a.ifc", 3)),
            severity: Severity::Error,
            message: "Door is not hosted by an opening".into(),
            related: vec![],
            evidence: vec![],
        }
        .with_related([id("a.ifc", 1)]),
    ];
    let export = export(&report, &model("a.ifc", 1), &options()).unwrap();
    assert!(export.document.topics[0].viewpoints.is_empty());
    assert_eq!(export.unanchored, [id("a.ifc", 3)]);
}

/// "The model has no building" and "at most one slab, found two" have no
/// subject object; the whole project has an outcome of its own.
fn scoped_report(document: &str, first: u64) -> Report {
    let source = SourceId::new("ifc-step", document).unwrap();
    Report {
        findings: vec![
            Finding::new(
                RuleId::new("building-exists").unwrap(),
                source.clone(),
                Severity::Error,
                "no object matches the selection; required at least 1",
            ),
            Finding::new(
                RuleId::new("one-wall-or-slab").unwrap(),
                source.clone(),
                Severity::Warning,
                "2 object(s) match the selection; required at most 1",
            )
            .with_related([id(document, first + 1), id(document, first)])
            .with_evidence([Evidence::exact(source.clone(), "selection:count")]),
        ],
        not_evaluated: vec![NotEvaluated {
            rule_id: RuleId::new("storey-exists").unwrap(),
            scope: Scope::Source(source),
            reason: NotEvaluatedReason::IncompleteEvidence,
            message: "0 object(s) match and 3 more may".into(),
        }],
    }
}

#[test]
fn a_source_or_project_outcome_is_a_model_level_topic_without_a_component() {
    let export = export(&scoped_report("a.ifc", 1), &model("a.ifc", 1), &options()).unwrap();
    assert!(export.unanchored.is_empty());
    let bytes = export.to_bytes().unwrap();
    assert!(viewpoints(&bytes).is_empty());
    let archive = openbim_bcf::read_slice(&bytes).unwrap();
    assert!(
        archive.diagnostics().is_empty(),
        "{:?}",
        archive.diagnostics()
    );
    let topics: Vec<_> = archive.topics().map(|markup| &markup.topic).collect();
    assert_eq!(topics.len(), 3);
    let described = |label: &str| {
        topics
            .iter()
            .find(|topic| topic.labels == [label])
            .and_then(|topic| topic.description.clone())
            .unwrap_or_else(|| panic!("no topic labelled {label}"))
    };
    let building = described("building-exists");
    assert!(
        building.contains("Source: ifc-step:a.ifc; no single object"),
        "{building}"
    );
    assert!(!building.contains("Object:"), "{building}");
    let count = described("one-wall-or-slab");
    // The objects found are named, not selected.
    assert!(
        count.contains("Related: ifc-step:a.ifc/#1, ifc-step:a.ifc/#2"),
        "{count}"
    );
    assert!(
        count.contains("Evidence (exact): selection:count"),
        "{count}"
    );
    let storey = described("storey-exists");
    assert!(
        storey.contains("Source: ifc-step:a.ifc; the rule was not evaluated for this source"),
        "{storey}"
    );
}

#[test]
fn a_project_finding_is_written_and_its_guid_is_stable() {
    let report = Report {
        findings: vec![Finding::new(
            RuleId::new("fire-compartment-exists").unwrap(),
            Scope::Project,
            Severity::Error,
            "no object matches the selection in the project; required at least 1",
        )],
        not_evaluated: vec![],
    };
    let guid = |project: &Project| {
        export(&report, project, &options())
            .unwrap()
            .document
            .topics[0]
            .guid
            .clone()
    };
    let topic = &export(&report, &model("a.ifc", 1), &options())
        .unwrap()
        .document
        .topics[0];
    assert!(topic.viewpoints.is_empty());
    assert!(
        topic
            .description
            .as_deref()
            .unwrap()
            .contains("Project: no single object or source")
    );
    assert_eq!(guid(&model("a.ifc", 1)), guid(&model("b.ifc", 7)));
}

#[test]
fn source_findings_keep_their_guids_across_renamed_revisions() {
    // A revised model saved under another name still reports the same
    // source-level issue; its GUID must not depend on the file name.
    let guids = |document: &str, first: u64| -> Vec<String> {
        export(
            &scoped_report(document, first),
            &model(document, first),
            &options(),
        )
        .unwrap()
        .document
        .topics
        .into_iter()
        .map(|topic| topic.guid)
        .collect()
    };
    assert_eq!(guids("a.ifc", 1), guids("a-rev2.ifc", 100));

    // Two sources in one project report the same issue: qualified, unique.
    let mut objects: Vec<_> = model("a.ifc", 1).objects().cloned().collect();
    objects.extend(model("b.ifc", 1).objects().cloned());
    let project = Project::new(objects).unwrap();
    let mut both = scoped_report("a.ifc", 1);
    let other = scoped_report("b.ifc", 1);
    both.findings.extend(other.findings);
    both.not_evaluated.extend(other.not_evaluated);
    let archive = openbim_bcf::read_slice(
        &export(&both, &project, &options())
            .unwrap()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(archive.topic_count(), 6);
}

#[test]
fn object_and_rule_level_topic_guids_are_unchanged_by_scopes() {
    // Pinned from before findings had a scope: a user's tracked issues keep
    // their GUIDs.
    let guids: Vec<String> = export(&report("a.ifc", 1), &model("a.ifc", 1), &options())
        .unwrap()
        .document
        .topics
        .into_iter()
        .map(|topic| topic.guid)
        .collect();
    assert_eq!(
        guids,
        [
            "ea43db6d-429c-568c-bbe4-f39f8c5b7d87",
            "2c812871-0333-5fb5-bf0c-4356042daf3a",
            "644f00fc-b113-537e-9d52-c298dbad2d72",
        ]
    );
}
