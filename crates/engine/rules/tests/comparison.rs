//! Model comparison contract tests.
//!
//! Two revisions of one model are exported with different local ids, as every
//! re-export renumbers. Matching must follow the external identity, and what
//! cannot be matched or read must be reported rather than dropped.
#![allow(missing_docs)]

use std::{collections::BTreeMap, sync::Arc};

use axioval_engine::{
    CompletePropertyAbsenceEvidence, EvidenceSession, PropertyRequest, PropertyResolution,
    PropertyResolutionError, PropertyResolutionService, PropertyResolutionServiceHandle,
    ResolvedProperty, SourceSnapshot,
};
use axioval_ir::{
    Classification, Evidence, ExternalId, NotEvaluatedReason, Object, ObjectId, Project, Property,
    PropertyValue, RuleId, Severity, SourceId,
};
use axioval_rules::{ComparisonRequest, Difference, ObjectChange, Side, compare_sessions};

const SCHEME: &str = "guid";

fn base_source() -> SourceId {
    SourceId::new("cad", "model-r1").unwrap()
}
fn revised_source() -> SourceId {
    SourceId::new("cad", "model-r2").unwrap()
}

/// An object with a local id that differs per revision and a stable guid.
fn object(source: &SourceId, local: &str, guid: Option<&str>, kind: &str) -> Object {
    let object = Object::new(ObjectId::new(source.clone(), local).unwrap(), kind);
    match guid {
        Some(guid) => object.with_external_id(ExternalId::new(SCHEME, guid).unwrap()),
        None => object,
    }
}

fn session(source: &SourceId, objects: Vec<Object>) -> EvidenceSession {
    let snapshot = SourceSnapshot::try_new(source.clone(), "r", "sha256:fixture").unwrap();
    EvidenceSession::try_new(Project::new(objects).unwrap(), [snapshot]).unwrap()
}

/// Resolves `FireRating` from a fixed table; absent elsewhere; errors for
/// objects listed as unreadable.
struct Ratings {
    values: BTreeMap<String, &'static str>,
    unreadable: Vec<String>,
    snapshots: Vec<SourceSnapshot>,
}

impl PropertyResolutionService for Ratings {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        let local = &request.object_id().local_id;
        if self.unreadable.contains(local) {
            return Err(PropertyResolutionError::Unavailable("fixture".into()));
        }
        let evidence = Evidence::exact(request.object_id().source.clone(), format!("#{local}"));
        match self.values.get(local) {
            Some(value) => Ok(PropertyResolution::Present(ResolvedProperty::try_new(
                request.clone(),
                Property::new(
                    "Pset_Common",
                    "FireRating",
                    PropertyValue::String((*value).into()),
                )
                .unwrap()
                .with_evidence(evidence),
            )?)),
            None => Ok(PropertyResolution::Absent(
                CompletePropertyAbsenceEvidence::try_new(request.clone(), evidence)?,
            )),
        }
    }
}

fn with_ratings(
    source: &SourceId,
    objects: Vec<Object>,
    values: &[(&str, &'static str)],
    unreadable: &[&str],
) -> EvidenceSession {
    let snapshot = SourceSnapshot::try_new(source.clone(), "r", "sha256:fixture").unwrap();
    EvidenceSession::try_new(Project::new(objects).unwrap(), [snapshot.clone()])
        .unwrap()
        .with_service(PropertyResolutionServiceHandle::new(Arc::new(Ratings {
            values: values.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
            unreadable: unreadable.iter().map(|s| (*s).to_owned()).collect(),
            snapshots: vec![snapshot],
        })))
        .unwrap()
}

fn request() -> ComparisonRequest {
    ComparisonRequest::new(SCHEME).unwrap()
}

#[test]
fn renumbered_objects_match_by_identity_and_report_additions_and_removals() {
    let base = session(
        &base_source(),
        vec![
            object(&base_source(), "#10", Some("A"), "wall"),
            object(&base_source(), "#11", Some("B"), "wall"),
        ],
    );
    let revised = session(
        &revised_source(),
        vec![
            object(&revised_source(), "#77", Some("A"), "wall"),
            object(&revised_source(), "#78", Some("C"), "door"),
        ],
    );
    let comparison = compare_sessions(&base, &revised, &request());
    let summary: Vec<(&str, &str)> = comparison
        .objects()
        .iter()
        .map(|compared| {
            let change = match &compared.change {
                ObjectChange::Added { .. } => "added",
                ObjectChange::Removed { .. } => "removed",
                ObjectChange::Matched { differences, .. } if differences.is_empty() => "same",
                ObjectChange::Matched { .. } => "changed",
            };
            (compared.identity.as_str(), change)
        })
        .collect();
    assert_eq!(
        summary,
        vec![("A", "same"), ("B", "removed"), ("C", "added")]
    );
    assert!(!comparison.is_identical());
}

#[test]
fn kind_classification_property_and_relationship_changes_are_reported() {
    let base = session(
        &base_source(),
        vec![
            object(&base_source(), "#1", Some("A"), "wall")
                .with_classification(Classification::new("Uniclass", "EF_25_10").unwrap())
                .with_property(
                    Property::new("Pset", "LoadBearing", PropertyValue::Boolean(true)).unwrap(),
                ),
            object(&base_source(), "#2", Some("S1"), "storey"),
            object(&base_source(), "#3", Some("S2"), "storey"),
        ],
    );
    let mut moved = object(&revised_source(), "#9", Some("A"), "curtain_wall")
        .with_classification(Classification::new("Uniclass", "EF_25_30").unwrap())
        .with_property(
            Property::new("Pset", "LoadBearing", PropertyValue::Boolean(false)).unwrap(),
        );
    moved.relationships.insert(
        "contained_in".into(),
        vec![ObjectId::new(revised_source(), "#8").unwrap()],
    );
    let mut base_objects: Vec<Object> = base.project().objects().cloned().collect();
    base_objects[0].relationships.insert(
        "contained_in".into(),
        vec![ObjectId::new(base_source(), "#2").unwrap()],
    );
    let base = session(&base_source(), base_objects);
    let revised = session(
        &revised_source(),
        vec![
            moved,
            object(&revised_source(), "#7", Some("S1"), "storey"),
            object(&revised_source(), "#8", Some("S2"), "storey"),
        ],
    );

    let comparison = compare_sessions(&base, &revised, &request());
    let ObjectChange::Matched {
        differences,
        unresolved,
        ..
    } = &comparison.objects()[0].change
    else {
        panic!("A is matched");
    };
    assert!(unresolved.is_empty(), "{unresolved:?}");
    let rendered: Vec<String> = differences.iter().map(ToString::to_string).collect();
    assert_eq!(
        rendered,
        vec![
            "kind wall -> curtain_wall",
            "classifications -[Uniclass:EF_25_10] +[Uniclass:EF_25_30]",
            "property Pset.LoadBearing true -> false",
            // Targets are named by identity, not by renumbered local id.
            "contained_in -[S1] +[S2]",
        ]
    );
}

#[test]
fn requested_properties_are_resolved_and_resolver_failures_are_unresolved() {
    let objects = |source: &SourceId| {
        vec![
            object(source, "a", Some("A"), "door"),
            object(source, "b", Some("B"), "door"),
            object(source, "c", Some("C"), "door"),
        ]
    };
    let base = with_ratings(
        &base_source(),
        objects(&base_source()),
        &[("a", "EI30"), ("b", "EI30")],
        &[],
    );
    let revised = with_ratings(
        &revised_source(),
        objects(&revised_source()),
        &[("a", "EI60"), ("b", "EI30")],
        &["c"],
    );
    let request = request()
        .with_property(Some("Pset_Common"), "FireRating")
        .unwrap();
    let comparison = compare_sessions(&base, &revised, &request);

    let changes: Vec<_> = comparison
        .objects()
        .iter()
        .map(|compared| match &compared.change {
            ObjectChange::Matched {
                differences,
                unresolved,
                ..
            } => (differences.len(), unresolved.len()),
            other => panic!("all matched: {other:?}"),
        })
        .collect();
    assert_eq!(changes, vec![(1, 0), (0, 0), (0, 1)]);
    assert!(matches!(
        &comparison.objects()[0].change,
        ObjectChange::Matched { differences, .. }
            if matches!(&differences[0], Difference::Property { base: Some(_), revised: Some(_), .. })
    ));

    let report = comparison.report(&RuleId::new("compare").unwrap(), &Severity::Info);
    assert_eq!(report.findings().len(), 1);
    assert_eq!(report.not_evaluated().len(), 1);
    assert_eq!(
        report.not_evaluated()[0].reason,
        NotEvaluatedReason::IncompleteEvidence
    );
}

/// Without a resolver there is no way to know a requested property's value.
/// Comparing "unknown" with "unknown" as equal would hide every change.
#[test]
fn a_session_without_a_resolver_leaves_requested_properties_unresolved() {
    let base = session(
        &base_source(),
        vec![object(&base_source(), "a", Some("A"), "door")],
    );
    let revised = session(
        &revised_source(),
        vec![object(&revised_source(), "a", Some("A"), "door")],
    );
    let request = request().with_property(None, "FireRating").unwrap();
    let comparison = compare_sessions(&base, &revised, &request);
    assert!(matches!(
        &comparison.objects()[0].change,
        ObjectChange::Matched { unresolved, .. } if unresolved.len() == 1
    ));
    assert!(!comparison.is_identical());
}

#[test]
fn unidentified_and_ambiguous_objects_are_reported_not_dropped() {
    let other = SourceId::new("cad", "linked").unwrap();
    let snapshots = [
        SourceSnapshot::try_new(revised_source(), "r", "sha256:a").unwrap(),
        SourceSnapshot::try_new(other.clone(), "r", "sha256:b").unwrap(),
    ];
    let revised = EvidenceSession::try_new(
        Project::new(vec![
            object(&revised_source(), "x", Some("A"), "wall"),
            // A federated copy claims the same identity from another source.
            object(&other, "y", Some("A"), "wall"),
            object(&revised_source(), "z", None, "wall"),
        ])
        .unwrap(),
        snapshots,
    )
    .unwrap();
    let base = session(
        &base_source(),
        vec![object(&base_source(), "a", Some("A"), "wall")],
    );

    let comparison = compare_sessions(&base, &revised, &request());
    assert!(
        comparison.objects().is_empty(),
        "an ambiguous identity matches nothing"
    );
    assert_eq!(
        comparison.unidentified(),
        &[(Side::Revised, ObjectId::new(revised_source(), "z").unwrap())]
    );
    let sides: Vec<(Side, usize)> = comparison
        .ambiguous()
        .iter()
        .map(|entry| (entry.side, entry.objects.len()))
        .collect();
    assert_eq!(sides, vec![(Side::Base, 1), (Side::Revised, 2)]);

    let report = comparison.report(&RuleId::new("compare").unwrap(), &Severity::Info);
    assert!(report.findings().is_empty());
    assert_eq!(report.not_evaluated().len(), 4);
}

#[test]
fn identical_sessions_are_identical() {
    let make = |source: &SourceId| {
        session(
            source,
            vec![object(source, "a", Some("A"), "wall").with_property(
                Property::new("Pset", "Width", PropertyValue::Decimal(0.0)).unwrap(),
            )],
        )
    };
    let revised = session(
        &revised_source(),
        vec![
            object(&revised_source(), "q", Some("A"), "wall").with_property(
                // Negative zero is the same width.
                Property::new("Pset", "Width", PropertyValue::Decimal(-0.0)).unwrap(),
            ),
        ],
    );
    assert!(compare_sessions(&make(&base_source()), &revised, &request()).is_identical());
}

#[test]
fn blank_declarations_are_refused() {
    assert!(ComparisonRequest::new(" ").is_err());
    assert!(request().with_property(Some(""), "x").is_err());
    assert!(request().with_property(None, " ").is_err());
}
