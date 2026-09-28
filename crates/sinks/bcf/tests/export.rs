//! Reports exported as BCF 2.1 and read back.
#![allow(missing_docs, clippy::doc_markdown)]

use std::io::{Cursor, Read};

use axioval_bcf::{
    ExportError, IFC_GLOBAL_ID_SCHEME, NOT_EVALUATED_TOPIC_TYPE, Options, PRIORITY_HIGH,
    PRIORITY_NORMAL, export,
};
use axioval_ir::{
    Evidence, ExternalId, Finding, NotEvaluated, NotEvaluatedReason, Object, ObjectId, Project,
    QuantityDimension, Report, ReportColumn, ReportTable, ReportValue, RuleId, Scope, Severity,
    SourceId,
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
        stale_decisions: Vec::new(),
        findings: vec![
            Finding {
                id: None,
                decision: None,
                rule_id: RuleId::new("slab-contact").unwrap(),
                scope: Scope::Object(id(document, first)),
                severity: Severity::Error,
                message: "Wall has insufficient contact with the slab below".into(),
                related: vec![],
                evidence: vec![Evidence::exact(source, "contact:wall")],
                location: None,
                categories: Vec::new(),
            }
            .with_related([id(document, first + 1)]),
            Finding {
                id: None,
                decision: None,
                rule_id: RuleId::new("door-fire-rating").unwrap(),
                scope: Scope::Object(id(document, first + 2)),
                severity: Severity::Warning,
                message: "FireRating is missing".into(),
                related: vec![],
                evidence: vec![],
                location: None,
                categories: Vec::new(),
            },
        ],
        not_evaluated: vec![NotEvaluated {
            rule_id: RuleId::new("stair-headroom").unwrap(),
            scope: Scope::Project,
            reason: NotEvaluatedReason::MissingService,
            message: "no geometry service is registered".into(),
            location: None,
        }],
        tables: vec![],
        rules: Vec::new(),
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
fn a_located_entry_is_labelled_by_storey_and_space_under_the_same_guid() {
    let plain = report("a.ifc", 1);
    let mut located = plain.clone();
    located.findings[0].location = Some(axioval_ir::Location {
        storeys: vec![axioval_ir::Place {
            id: id("a.ifc", 90),
            name: Some("Level 1".into()),
        }],
        spaces: vec![axioval_ir::Place {
            id: id("a.ifc", 91),
            name: None,
        }],
        unresolved: None,
    });
    let topics = |report: &Report| {
        export(report, &model("a.ifc", 1), &options())
            .unwrap()
            .document
            .topics
    };
    let (plain, located) = (topics(&plain), topics(&located));
    assert_eq!(plain[0].labels, ["slab-contact"]);
    assert_eq!(
        located[0].labels,
        [
            "slab-contact",
            "Storey: Level 1",
            "Space: ifc-step:a.ifc/#91"
        ]
    );
    assert_eq!(located[0].guid, plain[0].guid);

    // A rule's own labels go between the rule id and the location.
    let labelled = Options {
        rule_labels: [(
            "slab-contact".to_owned(),
            vec!["Folder: Structure".to_owned()],
        )]
        .into(),
        ..options()
    };
    let mut both = report("a.ifc", 1);
    both.findings[0].location = Some(axioval_ir::Location {
        storeys: vec![axioval_ir::Place {
            id: id("a.ifc", 90),
            name: Some("Level 1".into()),
        }],
        spaces: vec![],
        unresolved: None,
    });
    let topic = &export(&both, &model("a.ifc", 1), &labelled)
        .unwrap()
        .document
        .topics[0];
    assert_eq!(
        topic.labels,
        ["slab-contact", "Folder: Structure", "Storey: Level 1"]
    );

    // Its categories come after the rule's labels, before the location.
    both.findings[0].categories = vec!["F90".into(), "Office".into()];
    let topic = &export(&both, &model("a.ifc", 1), &labelled)
        .unwrap()
        .document
        .topics[0];
    assert_eq!(
        topic.labels,
        [
            "slab-contact",
            "Folder: Structure",
            "Category: F90 / Office",
            "Storey: Level 1"
        ]
    );
    assert_eq!(topic.guid, located[0].guid);
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
    assert_eq!(wall.priority.as_deref(), Some(PRIORITY_HIGH));
    assert_eq!(
        by_title("FireRating is missing").priority.as_deref(),
        Some(PRIORITY_NORMAL)
    );
    let description = wall.description.as_deref().unwrap();
    assert!(
        description.contains("Evidence (exact): contact:wall"),
        "{description}"
    );

    let skipped = by_title("no geometry service is registered");
    // Nothing was decided about it, so no priority is claimed.
    assert_eq!(skipped.priority, None);
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
fn report_tables_write_no_topics() {
    let bytes = |report: &Report| {
        export(report, &model("a.ifc", 1), &options())
            .unwrap()
            .to_bytes()
            .unwrap()
    };
    let plain = report("a.ifc", 1);
    let mut tabled = plain.clone();
    let mut table = ReportTable::new(
        RuleId::new("storey-height").unwrap(),
        "levels",
        vec![ReportColumn::quantity("height", QuantityDimension::Length)],
    )
    .unwrap();
    table
        .push_row(id("a.ifc", 1), vec![ReportValue::exact(3.0)])
        .unwrap();
    tabled.tables.push(table);
    assert_eq!(bytes(&tabled), bytes(&plain));
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
            id: None,
            decision: None,
            rule_id: RuleId::new("door-in-wall").unwrap(),
            scope: Scope::Object(id("a.ifc", 3)),
            severity: Severity::Error,
            message: "Door is not hosted by an opening".into(),
            related: vec![],
            evidence: vec![],
            location: None,
            categories: Vec::new(),
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
        stale_decisions: Vec::new(),
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
            location: None,
        }],
        tables: vec![],
        rules: Vec::new(),
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
        stale_decisions: Vec::new(),
        findings: vec![Finding::new(
            RuleId::new("fire-compartment-exists").unwrap(),
            Scope::Project,
            Severity::Error,
            "no object matches the selection in the project; required at least 1",
        )],
        not_evaluated: vec![],
        tables: vec![],
        rules: Vec::new(),
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

mod cameras {
    use std::collections::BTreeMap;

    use axioval_bcf::{
        Bounds, ExportError, FIELD_OF_VIEW_DEGREES, MIN_FRAME_RADIUS_METRES, Options, Version,
        export,
    };
    use axioval_ir::ObjectId;
    use openbim_bcf::write::{Camera, Projection};

    use super::{SLAB, WALL, id, model, options, report, viewpoints};

    /// The wall stands on the slab: together they span 0..4 x 0..2 x -0.2..3.
    pub(super) fn bounds() -> BTreeMap<ObjectId, Bounds> {
        BTreeMap::from([
            (
                id("a.ifc", 1),
                Bounds::new([1.0, 0.0, 0.0], [3.0, 0.2, 3.0]).unwrap(),
            ),
            (
                id("a.ifc", 2),
                Bounds::new([0.0, 0.0, -0.2], [4.0, 2.0, 0.0]).unwrap(),
            ),
        ])
    }

    /// Half the diagonal of the union of [`bounds`].
    fn half_diagonal() -> f64 {
        (4.0f64 * 4.0 + 2.0 * 2.0 + 3.2 * 3.2).sqrt() / 2.0
    }

    fn measured(version: Version) -> Options {
        Options {
            version,
            bounds: Some(bounds()),
            ..options()
        }
    }

    /// The camera looks at `centre` from above, far enough for the union's
    /// bounding sphere to fit its field of view, with a perpendicular up.
    fn assert_frames(camera: &Camera, centre: [f64; 3]) {
        let d = camera.direction;
        let length = (d.x * d.x + d.y * d.y + d.z * d.z).sqrt();
        let unit = [d.x / length, d.y / length, d.z / length];
        let to_centre = [
            centre[0] - camera.view_point.x,
            centre[1] - camera.view_point.y,
            centre[2] - camera.view_point.z,
        ];
        let along: f64 = (0..3).map(|axis| to_centre[axis] * unit[axis]).sum();
        let off = (0..3)
            .map(|axis| (to_centre[axis] - unit[axis] * along).powi(2))
            .sum::<f64>()
            .sqrt();
        assert!(off < 1e-5, "{camera:?} misses the centre by {off}");
        assert!(d.z < 0.0 && camera.view_point.z > centre[2], "{camera:?}");
        let half_angle = (FIELD_OF_VIEW_DEGREES / 2.0).to_radians();
        assert!(along * half_angle.sin() >= half_diagonal(), "{camera:?}");
        let u = camera.up_vector;
        assert!(
            (u.x * d.x + u.y * d.y + u.z * d.z).abs() < 1e-5,
            "{camera:?}"
        );
        assert!(u.z > 0.0, "{camera:?}");
    }

    #[test]
    fn bounded_objects_get_a_perspective_and_an_orthogonal_camera() {
        let plain = export(&report("a.ifc", 1), &model("a.ifc", 1), &options()).unwrap();
        let export = export(
            &report("a.ifc", 1),
            &model("a.ifc", 1),
            &measured(Version::V2_1),
        )
        .unwrap();
        assert!(export.unframed.is_empty(), "{:?}", export.unframed);

        let topic = &export.document.topics[0];
        let [perspective, orthogonal] = topic.viewpoints.as_slice() else {
            panic!("{:?}", topic.viewpoints);
        };
        // The first viewpoint keeps the GUID it had without a camera.
        assert_eq!(
            perspective.guid,
            plain.document.topics[0].viewpoints[0].guid
        );
        assert_eq!(perspective.selection, orthogonal.selection);

        let centre = [2.0, 1.0, 1.4];
        let camera = perspective.camera.unwrap();
        assert_eq!(
            camera.projection,
            Projection::Perspective {
                field_of_view: FIELD_OF_VIEW_DEGREES
            }
        );
        assert_eq!(camera.aspect_ratio, None, "2.1 has no aspect ratio");
        assert_frames(&camera, centre);
        let camera = orthogonal.camera.unwrap();
        let Projection::Orthogonal {
            view_to_world_scale,
        } = camera.projection
        else {
            panic!("{camera:?}");
        };
        assert!(view_to_world_scale >= 2.0 * half_diagonal(), "{camera:?}");
        assert_frames(&camera, centre);

        let bytes = export.to_bytes().unwrap();
        let archive = openbim_bcf::read_slice(&bytes).unwrap();
        assert!(
            archive.diagnostics().is_empty(),
            "{:?}",
            archive.diagnostics()
        );
        let views = viewpoints(&bytes);
        assert_eq!(views.len(), 2, "{views:?}");
        assert!(
            views
                .iter()
                .any(|view| view.contains("<PerspectiveCamera>"))
        );
        assert!(views.iter().any(|view| view.contains("<OrthogonalCamera>")));
        assert!(
            views
                .iter()
                .all(|view| view.contains(WALL) && view.contains(SLAB))
        );
    }

    #[test]
    fn a_missing_bound_leaves_the_viewpoint_without_a_camera() {
        let mut partial = bounds();
        partial.remove(&id("a.ifc", 2));
        let options = Options {
            bounds: Some(partial),
            ..options()
        };
        let export = export(&report("a.ifc", 1), &model("a.ifc", 1), &options).unwrap();
        let topic = &export.document.topics[0];
        assert_eq!(topic.viewpoints.len(), 1);
        assert_eq!(topic.viewpoints[0].camera, None);
        assert_eq!(export.unframed, [id("a.ifc", 2)]);
    }

    #[test]
    fn without_bounds_no_camera_is_written_and_nothing_is_unframed() {
        let unmeasured = options();
        assert_eq!(unmeasured.bounds, None);
        let export = export(&report("a.ifc", 1), &model("a.ifc", 1), &unmeasured).unwrap();
        assert!(export.unframed.is_empty());
        let views = viewpoints(&export.to_bytes().unwrap());
        assert_eq!(views.len(), 1);
        assert!(!views[0].contains("Camera"), "{}", views[0]);
    }

    #[test]
    fn bcf_3_is_written_when_every_viewpoint_has_a_camera() {
        let export = export(
            &report("a.ifc", 1),
            &model("a.ifc", 1),
            &measured(Version::V3_0),
        )
        .unwrap();
        let camera = export.document.topics[0].viewpoints[0].camera.unwrap();
        assert!(camera.aspect_ratio.is_some(), "3.0 requires it");
        let bytes = export.to_bytes().unwrap();
        let archive = openbim_bcf::read_slice(&bytes).unwrap();
        assert_eq!(
            archive.version().resolved(),
            Some(openbim_bcf::BcfVersion::V3_0)
        );
        assert!(
            archive.diagnostics().is_empty(),
            "{:?}",
            archive.diagnostics()
        );
        // The door (no GlobalId) and the project outcome have no viewpoint,
        // so they need no camera.
        assert_eq!(archive.topic_count(), 3);
    }

    #[test]
    fn bcf_3_without_a_camera_is_refused() {
        let error = export(
            &report("a.ifc", 1),
            &model("a.ifc", 1),
            &Options {
                version: Version::V3_0,
                ..options()
            },
        )
        .unwrap_err();
        assert!(
            matches!(&error, ExportError::MissingCamera { object } if *object == id("a.ifc", 1)),
            "{error}"
        );

        let mut partial = bounds();
        partial.remove(&id("a.ifc", 2));
        let error = export(
            &report("a.ifc", 1),
            &model("a.ifc", 1),
            &Options {
                bounds: Some(partial),
                ..measured(Version::V3_0)
            },
        )
        .unwrap_err();
        assert!(
            matches!(&error, ExportError::MissingCamera { object } if *object == id("a.ifc", 2)),
            "{error}"
        );
    }

    #[test]
    fn bounds_refuse_inverted_or_infinite_corners() {
        assert!(Bounds::new([1.0, 0.0, 0.0], [0.0, 1.0, 1.0]).is_none());
        assert!(Bounds::new([0.0, 0.0, f64::NAN], [1.0, 1.0, 1.0]).is_none());
        assert!(Bounds::new([0.0; 3], [0.0; 3]).is_some());
    }

    #[test]
    fn a_point_is_framed_with_its_surroundings() {
        let point = Bounds::new([5.0; 3], [5.0; 3]).unwrap();
        let options = Options {
            bounds: Some(BTreeMap::from([
                (id("a.ifc", 1), point),
                (id("a.ifc", 2), point),
            ])),
            ..options()
        };
        let export = export(&report("a.ifc", 1), &model("a.ifc", 1), &options).unwrap();
        let camera = export.document.topics[0].viewpoints[1].camera.unwrap();
        assert_eq!(
            camera.projection,
            Projection::Orthogonal {
                view_to_world_scale: 2.0 * MIN_FRAME_RADIUS_METRES
            }
        );
    }
}

mod labels {
    use axioval_bcf::{Options, export, ruleset_labels};
    use axioval_ir::RuleSetPackage;
    use serde_json::{Value, json};

    use super::{model, options, report};

    /// The schema fixture's ruleset with its rule renamed `slab-contact`,
    /// nested in `Structure / Walls` and tagged, beside a root rule without
    /// tags.
    fn ruleset() -> RuleSetPackage {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../fixtures/schema-v0.1.0/ruleset.json"
        ))
        .unwrap();
        let mut ruleset: Value = serde_json::from_str(&text).unwrap();
        let mut rule = ruleset["root"]["rules"][0].clone();
        rule["id"] = json!("slab-contact");
        rule["tags"] = json!(["structure", " ", "example"]);
        let mut untagged = rule.clone();
        untagged["id"] = json!("door-fire-rating");
        untagged["tags"] = json!([]);
        ruleset["root"]["rules"] = json!([untagged]);
        ruleset["root"]["folders"] = json!([{
            "id": "structure",
            "name": {"default": "Structure", "translations": {}},
            "rules": [],
            "folders": [{
                "id": "walls",
                "name": {"default": " Walls ", "translations": {}},
                "rules": [rule],
                "folders": [],
            }],
        }]);
        serde_json::from_value(ruleset).unwrap()
    }

    #[test]
    fn a_ruleset_labels_its_rules_by_folder_path_and_tags() {
        let labels = ruleset_labels(&ruleset());
        assert_eq!(
            labels.get("slab-contact").unwrap(),
            &["Folder: Structure / Walls", "structure", "example"]
        );
        // A root rule without tags has no labels of its own.
        assert!(!labels.contains_key("door-fire-rating"), "{labels:?}");
    }

    #[test]
    fn topics_carry_the_rule_labels_after_the_rule_id_under_unchanged_guids() {
        let plain = export(&report("a.ifc", 1), &model("a.ifc", 1), &options()).unwrap();
        let labelled = Options {
            rule_labels: ruleset_labels(&ruleset()),
            ..options()
        };
        let export = export(&report("a.ifc", 1), &model("a.ifc", 1), &labelled).unwrap();
        let archive = openbim_bcf::read_slice(&export.to_bytes().unwrap()).unwrap();
        assert!(
            archive.diagnostics().is_empty(),
            "{:?}",
            archive.diagnostics()
        );
        let labels: Vec<Vec<String>> = archive
            .topics()
            .map(|markup| markup.topic.labels.clone())
            .collect();
        assert!(
            labels.contains(&vec![
                "slab-contact".to_owned(),
                "Folder: Structure / Walls".to_owned(),
                "structure".to_owned(),
                "example".to_owned(),
            ]),
            "{labels:?}"
        );
        assert!(labels.contains(&vec!["door-fire-rating".to_owned()]));
        let guids = |export: &axioval_bcf::Export| -> Vec<String> {
            export
                .document
                .topics
                .iter()
                .map(|topic| topic.guid.clone())
                .collect()
        };
        assert_eq!(guids(&export), guids(&plain));
    }
}

/// Decisions carried over to a report become topic statuses and comments.
mod decisions {
    use super::{export, model, options, report};
    use axioval_bcf::{
        DECISION_CHANGED_LABEL, IFC_GLOBAL_ID_SCHEME, Options, STATUS_ACCEPTED, STATUS_REJECTED,
    };
    use axioval_ir::{Decision, DecisionStatus, Decisions, Report, Severity};

    /// The wall finding accepted with a comment, the door finding rejected
    /// without one, each recorded against the report as it is.
    fn decided(report: &mut Report, date: &str) -> Decisions {
        report
            .identify_findings(&model("a.ifc", 1), IFC_GLOBAL_ID_SCHEME)
            .unwrap();
        let date = date.parse().unwrap();
        let wall = &report.findings[0];
        let door = &report.findings[1];
        Decisions::new([
            Decision::new(
                wall.id.unwrap(),
                DecisionStatus::Accepted,
                "A. Reviewer",
                date,
            )
            .unwrap()
            .with_comment("  agreed with the structural engineer ")
            .with_basis(wall),
            Decision::new(
                door.id.unwrap(),
                DecisionStatus::Rejected,
                "B. Reviewer",
                date,
            )
            .unwrap()
            .with_basis(door),
        ])
        .unwrap()
    }

    #[test]
    fn topic_guids_are_the_finding_identities() {
        let mut report = report("a.ifc", 1);
        decided(&mut report, "2026-09-27T08:00:00Z");
        let export = export(&report, &model("a.ifc", 1), &options()).unwrap();
        for (finding, topic) in report.findings.iter().zip(&export.document.topics) {
            assert_eq!(finding.id.unwrap().to_string(), topic.guid);
        }
    }

    #[test]
    fn a_decision_sets_the_topic_status_and_adds_its_comment() {
        let mut report = report("a.ifc", 1);
        let decisions = decided(&mut report, "2026-09-27T08:00:00+02:00");
        report.apply_decisions(&decisions).unwrap();
        let export = export(&report, &model("a.ifc", 1), &options()).unwrap();
        let bytes = export.to_bytes().unwrap();
        let archive = openbim_bcf::read_slice(&bytes).unwrap();
        assert!(
            archive.diagnostics().is_empty(),
            "{:?}",
            archive.diagnostics()
        );
        let markups: Vec<_> = archive.topics().collect();
        let wall = markups
            .iter()
            .find(|markup| markup.topic.title.as_deref().unwrap().starts_with("Wall"))
            .unwrap();
        assert_eq!(wall.topic.topic_status.as_deref(), Some(STATUS_ACCEPTED));
        assert_eq!(wall.comments.len(), 1);
        let comment = &wall.comments[0];
        assert_eq!(
            comment.comment.as_deref(),
            Some("Accepted: agreed with the structural engineer")
        );
        assert_eq!(comment.author.as_deref(), Some("A. Reviewer"));
        assert_eq!(comment.date.as_deref(), Some("2026-09-27T08:00:00+02:00"));
        // The comment GUID derives from the topic's, so it is reproduced.
        let topic_guid: uuid::Uuid = wall.topic.guid.as_deref().unwrap().parse().unwrap();
        assert_eq!(
            comment.guid.as_deref(),
            Some(
                uuid::Uuid::new_v5(&topic_guid, b"decision")
                    .to_string()
                    .as_str()
            )
        );

        let door = markups
            .iter()
            .find(|markup| markup.topic.title.as_deref() == Some("FireRating is missing"))
            .unwrap();
        assert_eq!(door.topic.topic_status.as_deref(), Some(STATUS_REJECTED));
        assert_eq!(door.comments[0].comment.as_deref(), Some("Rejected"));

        // A not-evaluated outcome is never decided: it stays open.
        let skipped = markups
            .iter()
            .find(|markup| {
                markup.topic.title.as_deref() == Some("no geometry service is registered")
            })
            .unwrap();
        assert_eq!(skipped.topic.topic_status.as_deref(), Some("Open"));
        assert!(skipped.comments.is_empty());
    }

    #[test]
    fn a_changed_finding_is_labelled_and_its_comment_says_what_changed() {
        let mut report = report("a.ifc", 1);
        let decisions = decided(&mut report, "2026-09-27T08:00:00Z");
        report.findings[0].severity = Severity::Warning;
        report.apply_decisions(&decisions).unwrap();
        let export = export(&report, &model("a.ifc", 1), &options()).unwrap();
        let wall = &export.document.topics[0];
        assert_eq!(wall.topic_status.as_deref(), Some(STATUS_ACCEPTED));
        assert_eq!(
            wall.labels,
            ["slab-contact".to_owned(), DECISION_CHANGED_LABEL.to_owned()]
        );
        assert_eq!(
            wall.comments[0].comment,
            "Accepted: agreed with the structural engineer\nChanged since the decision: severity error -> warning"
        );
        export.to_bytes().unwrap();
    }

    #[test]
    fn an_open_decision_keeps_the_hosts_status() {
        let mut report = report("a.ifc", 1);
        let mut decisions = decided(&mut report, "2026-09-27T08:00:00Z");
        let wall = report.findings[0].id.unwrap();
        let open = Decision::new(
            wall,
            DecisionStatus::Open,
            "A. Reviewer",
            "2026-09-28T08:00:00Z".parse().unwrap(),
        )
        .unwrap();
        decisions.record(open).unwrap();
        report.apply_decisions(&decisions).unwrap();
        let options = Options {
            status: "Active".to_owned(),
            ..options()
        };
        let export = export(&report, &model("a.ifc", 1), &options).unwrap();
        assert_eq!(
            export.document.topics[0].topic_status.as_deref(),
            Some("Active")
        );
        assert_eq!(export.document.topics[0].comments[0].comment, "Open");
    }

    #[test]
    fn without_decisions_the_archive_is_byte_identical() {
        let plain = report("a.ifc", 1);
        let mut identified = plain.clone();
        identified
            .identify_findings(&model("a.ifc", 1), IFC_GLOBAL_ID_SCHEME)
            .unwrap();
        identified.apply_decisions(&Decisions::default()).unwrap();
        let bytes = |report: &Report| {
            export(report, &model("a.ifc", 1), &options())
                .unwrap()
                .to_bytes()
                .unwrap()
        };
        assert_eq!(bytes(&plain), bytes(&identified));
    }
}

mod coloring {
    use axioval_bcf::{Color, Colors, Options, RELATED_COLOR, SUBJECT_COLOR, Version, export};
    use openbim_bcf::Component;
    use openbim_bcf::write::Coloring;

    use super::{SLAB, WALL, model, options, report, viewpoints};

    fn colored(colors: Colors) -> Options {
        Options {
            colors: Some(colors),
            ..options()
        }
    }

    #[test]
    fn the_subject_and_the_related_objects_are_coloured_apart() {
        let export = export(
            &report("a.ifc", 1),
            &model("a.ifc", 1),
            &colored(Colors::default()),
        )
        .unwrap();
        let view = &export.document.topics[0].viewpoints[0];
        assert_eq!(
            view.coloring,
            [
                Coloring {
                    color: "FFFF0000".into(),
                    components: vec![Component::ifc(WALL)],
                },
                Coloring {
                    color: "FF0000FF".into(),
                    components: vec![Component::ifc(SLAB)],
                },
            ]
        );
        // Selection and GUID are those of an uncoloured export.
        let plain = super::export(&report("a.ifc", 1), &model("a.ifc", 1), &options()).unwrap();
        let plain = &plain.document.topics[0].viewpoints[0];
        assert_eq!(
            (&view.guid, &view.selection),
            (&plain.guid, &plain.selection)
        );

        let bytes = export.to_bytes().unwrap();
        let archive = openbim_bcf::read_slice(&bytes).unwrap();
        assert!(
            archive.diagnostics().is_empty(),
            "{:?}",
            archive.diagnostics()
        );
        let views = viewpoints(&bytes);
        assert!(
            views[0].contains("<Color Color=\"FFFF0000\">")
                && views[0].contains("<Color Color=\"FF0000FF\">"),
            "{}",
            views[0]
        );
    }

    #[test]
    fn a_subject_without_related_objects_is_coloured_alone() {
        let mut report = report("a.ifc", 1);
        report.findings[0].related.clear();
        let export = export(&report, &model("a.ifc", 1), &colored(Colors::default())).unwrap();
        let view = &export.document.topics[0].viewpoints[0];
        assert_eq!(view.coloring.len(), 1, "{:?}", view.coloring);
        assert_eq!(view.coloring[0].color, SUBJECT_COLOR.to_string());
    }

    #[test]
    fn configured_colours_are_written_in_every_viewpoint_of_both_versions() {
        let colors = Colors {
            subject: "00ff00".parse().unwrap(),
            related: "80FFA500".parse().unwrap(),
        };
        for version in [Version::V2_1, Version::V3_0] {
            let options = Options {
                version,
                bounds: Some(super::cameras::bounds()),
                ..colored(colors)
            };
            let export = export(&report("a.ifc", 1), &model("a.ifc", 1), &options).unwrap();
            let topic = &export.document.topics[0];
            assert_eq!(topic.viewpoints.len(), 2);
            for view in &topic.viewpoints {
                let colors: Vec<&str> = view.coloring.iter().map(|c| c.color.as_str()).collect();
                assert_eq!(colors, ["FF00FF00", "80FFA500"]);
            }
            let bytes = export.to_bytes().unwrap();
            assert!(
                openbim_bcf::read_slice(&bytes)
                    .unwrap()
                    .diagnostics()
                    .is_empty()
            );
            assert_eq!(bytes, export.to_bytes().unwrap());
        }
    }

    #[test]
    fn without_colours_nothing_is_coloured() {
        assert_eq!(options().colors, None);
        let export = export(&report("a.ifc", 1), &model("a.ifc", 1), &options()).unwrap();
        let views = viewpoints(&export.to_bytes().unwrap());
        assert!(!views[0].contains("Coloring"), "{}", views[0]);
    }

    #[test]
    fn colours_parse_from_six_or_eight_hex_digits() {
        assert_eq!("ff0000".parse::<Color>(), Ok(SUBJECT_COLOR));
        assert_eq!("FF0000FF".parse::<Color>(), Ok(RELATED_COLOR));
        assert_eq!(Color::argb(0x0012_abcd).to_string(), "0012ABCD");
        for bad in [
            "",
            "FF00",
            "FF00000",
            "FF0000FF0",
            "+F0000",
            "GG0000",
            "ff 000",
        ] {
            assert!(bad.parse::<Color>().is_err(), "{bad:?}");
        }
    }
}

mod visibility {
    use axioval_bcf::{Options, Version, export};
    use openbim_bcf::Component;
    use openbim_bcf::write::Visibility;

    use super::{SLAB, WALL, model, options, report, viewpoints};

    #[test]
    fn isolated_viewpoints_hide_everything_but_the_involved_objects() {
        for version in [Version::V2_1, Version::V3_0] {
            let options = Options {
                version,
                isolate: true,
                bounds: Some(super::cameras::bounds()),
                ..options()
            };
            let export = export(&report("a.ifc", 1), &model("a.ifc", 1), &options).unwrap();
            let topic = &export.document.topics[0];
            assert_eq!(topic.viewpoints.len(), 2);
            for view in &topic.viewpoints {
                assert_eq!(
                    view.visibility,
                    Some(Visibility {
                        default_visibility: false,
                        exceptions: vec![Component::ifc(WALL), Component::ifc(SLAB)],
                    })
                );
            }
            let bytes = export.to_bytes().unwrap();
            let archive = openbim_bcf::read_slice(&bytes).unwrap();
            assert!(
                archive.diagnostics().is_empty(),
                "{:?}",
                archive.diagnostics()
            );
            for view in viewpoints(&bytes) {
                assert!(
                    view.contains("DefaultVisibility=\"false\"") && view.contains("<Exceptions>"),
                    "{view}"
                );
            }
        }
    }

    #[test]
    fn by_default_the_whole_model_stays_visible() {
        assert!(!options().isolate);
        let export = export(&report("a.ifc", 1), &model("a.ifc", 1), &options()).unwrap();
        assert_eq!(export.document.topics[0].viewpoints[0].visibility, None);
        let views = viewpoints(&export.to_bytes().unwrap());
        assert!(
            views[0].contains("<Visibility DefaultVisibility=\"true\"/>")
                && !views[0].contains("Exceptions"),
            "{}",
            views[0]
        );
    }

    #[test]
    fn a_topic_without_a_viewpoint_stays_without_one_when_isolated() {
        let options = Options {
            isolate: true,
            ..options()
        };
        let export = export(&report("a.ifc", 1), &model("a.ifc", 1), &options).unwrap();
        // The door has no GlobalId and the project outcome no subject.
        assert!(export.document.topics[1].viewpoints.is_empty());
        assert!(export.document.topics[2].viewpoints.is_empty());
    }
}

mod section_box {
    use axioval_bcf::{Options, Version, export};
    use openbim_bcf::write::{ClippingPlane, Vector3};

    use super::{id, model, options, report, viewpoints};

    fn boxed(version: Version) -> Options {
        Options {
            version,
            section_box: true,
            bounds: Some(super::cameras::bounds()),
            ..options()
        }
    }

    fn plane(location: [f64; 3], direction: [f64; 3]) -> ClippingPlane {
        ClippingPlane {
            location: Vector3::new(location[0], location[1], location[2]),
            direction: Vector3::new(direction[0], direction[1], direction[2]),
        }
    }

    #[test]
    fn a_framed_viewpoint_is_cut_by_a_box_around_its_objects() {
        for version in [Version::V2_1, Version::V3_0] {
            let export = export(&report("a.ifc", 1), &model("a.ifc", 1), &boxed(version)).unwrap();
            let topic = &export.document.topics[0];
            assert_eq!(topic.viewpoints.len(), 2);
            // The union spans 0..4 x 0..2 x -0.2..3; the box reaches half a
            // metre beyond it, each plane pointing outwards.
            let expected = [
                plane([-0.5, 1.0, 1.4], [-1.0, 0.0, 0.0]),
                plane([4.5, 1.0, 1.4], [1.0, 0.0, 0.0]),
                plane([2.0, -0.5, 1.4], [0.0, -1.0, 0.0]),
                plane([2.0, 2.5, 1.4], [0.0, 1.0, 0.0]),
                plane([2.0, 1.0, -0.7], [0.0, 0.0, -1.0]),
                plane([2.0, 1.0, 3.5], [0.0, 0.0, 1.0]),
            ];
            for view in &topic.viewpoints {
                assert_eq!(view.clipping_planes, expected);
            }
            let bytes = export.to_bytes().unwrap();
            let archive = openbim_bcf::read_slice(&bytes).unwrap();
            assert!(
                archive.diagnostics().is_empty(),
                "{:?}",
                archive.diagnostics()
            );
            assert!(
                viewpoints(&bytes)
                    .iter()
                    .all(|view| view.matches("<ClippingPlane>").count() == 6),
            );
            assert_eq!(bytes, export.to_bytes().unwrap());
        }
    }

    #[test]
    fn a_viewpoint_without_a_camera_is_never_clipped() {
        let unmeasured = Options {
            section_box: true,
            ..options()
        };
        let unframed = export(&report("a.ifc", 1), &model("a.ifc", 1), &unmeasured).unwrap();
        assert!(
            unframed.document.topics[0].viewpoints[0]
                .clipping_planes
                .is_empty()
        );

        let mut partial = super::cameras::bounds();
        partial.remove(&id("a.ifc", 2));
        let options = Options {
            bounds: Some(partial),
            ..boxed(Version::V2_1)
        };
        let export = export(&report("a.ifc", 1), &model("a.ifc", 1), &options).unwrap();
        let view = &export.document.topics[0].viewpoints[0];
        assert_eq!(view.camera, None);
        assert!(view.clipping_planes.is_empty());
    }

    #[test]
    fn without_a_section_box_nothing_is_clipped() {
        assert!(!options().section_box);
        let options = Options {
            section_box: false,
            ..boxed(Version::V2_1)
        };
        let export = export(&report("a.ifc", 1), &model("a.ifc", 1), &options).unwrap();
        let views = viewpoints(&export.to_bytes().unwrap());
        assert!(views.iter().all(|view| !view.contains("ClippingPlanes")));
    }
}
