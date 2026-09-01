//! Source/evidence session contract tests.

use axioval_engine::{
    EvidenceSession, EvidenceSessionError, ServiceRegistryError, SnapshotBoundService,
    SourceSnapshot,
};
use axioval_ir::{Object, ObjectId, Project, SourceId};

#[derive(Clone, Debug, Eq, PartialEq)]
struct Marker {
    name: &'static str,
    snapshots: Vec<SourceSnapshot>,
}

impl SnapshotBoundService for Marker {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }
}

fn project() -> Project {
    let source = SourceId::new("test", "snapshot-1").unwrap();
    let object = Object::new(ObjectId::new(source, "42").unwrap(), "WALL");
    Project::new(vec![object]).unwrap()
}

fn snapshot() -> SourceSnapshot {
    SourceSnapshot::try_new(
        SourceId::new("test", "snapshot-1").unwrap(),
        "revision-7",
        "sha256:0123456789abcdef",
    )
    .unwrap()
    .with_schema("TEST-SCHEMA-1")
    .unwrap()
}

fn marker(name: &'static str, snapshots: Vec<SourceSnapshot>) -> Marker {
    Marker { name, snapshots }
}

#[test]
fn session_binds_an_immutable_project_snapshot_to_services() {
    let session = EvidenceSession::try_new(project(), [snapshot()])
        .unwrap()
        .with_service(marker("exact-snapshot", vec![snapshot()]))
        .unwrap();

    assert_eq!(session.project().objects().count(), 1);
    assert_eq!(
        session.service::<Marker>(),
        Some(&marker("exact-snapshot", vec![snapshot()]))
    );
    assert_eq!(session.snapshots().len(), 1);
    assert_eq!(
        session.snapshots().next().unwrap().schema(),
        Some("TEST-SCHEMA-1")
    );
}

#[test]
fn session_refuses_silent_service_replacement() {
    let duplicate = EvidenceSession::try_new(project(), [snapshot()])
        .unwrap()
        .with_service(marker("first", vec![snapshot()]))
        .unwrap()
        .with_service(marker("second", vec![snapshot()]));

    assert!(matches!(
        duplicate,
        Err(EvidenceSessionError::ServiceRegistry(
            ServiceRegistryError::Duplicate
        ))
    ));
}

#[test]
fn session_rejects_unbound_stale_duplicate_and_unknown_service_bindings() {
    let unbound = EvidenceSession::try_new(project(), [snapshot()])
        .unwrap()
        .with_service(marker("unbound", vec![]));
    assert!(matches!(unbound, Err(EvidenceSessionError::UnboundService)));
    let empty_unbound = EvidenceSession::try_new(Project::new(vec![]).unwrap(), [])
        .unwrap()
        .with_service(marker("empty-unbound", vec![]));
    assert!(matches!(
        empty_unbound,
        Err(EvidenceSessionError::UnboundService)
    ));

    let stale_bindings = [
        SourceSnapshot::try_new(
            snapshot().source().clone(),
            "revision-stale",
            snapshot().fingerprint(),
        )
        .unwrap()
        .with_schema(snapshot().schema().unwrap())
        .unwrap(),
        SourceSnapshot::try_new(
            snapshot().source().clone(),
            snapshot().revision(),
            "sha256:stale",
        )
        .unwrap()
        .with_schema(snapshot().schema().unwrap())
        .unwrap(),
        SourceSnapshot::try_new(
            snapshot().source().clone(),
            snapshot().revision(),
            snapshot().fingerprint(),
        )
        .unwrap()
        .with_schema("TEST-SCHEMA-STALE")
        .unwrap(),
    ];
    for stale in stale_bindings {
        let mismatch = EvidenceSession::try_new(project(), [snapshot()])
            .unwrap()
            .with_service(marker("stale", vec![stale]));
        assert!(matches!(
            mismatch,
            Err(EvidenceSessionError::ServiceSnapshotMismatch(_))
        ));
    }

    let duplicate = EvidenceSession::try_new(project(), [snapshot()])
        .unwrap()
        .with_service(marker("duplicate", vec![snapshot(), snapshot()]));
    assert!(matches!(
        duplicate,
        Err(EvidenceSessionError::DuplicateServiceSource(_))
    ));

    let unknown = SourceSnapshot::try_new(
        SourceId::new("test", "other").unwrap(),
        "revision-7",
        "sha256:0123456789abcdef",
    )
    .unwrap();
    let mismatch = EvidenceSession::try_new(project(), [snapshot()])
        .unwrap()
        .with_service(marker("unknown", vec![unknown]));
    assert!(matches!(
        mismatch,
        Err(EvidenceSessionError::ServiceSnapshotMismatch(_))
    ));
}

#[test]
fn session_rejects_missing_duplicate_and_unexpected_snapshots() {
    assert!(matches!(
        EvidenceSession::try_new(project(), []),
        Err(EvidenceSessionError::MissingSource(_))
    ));
    assert!(matches!(
        EvidenceSession::try_new(project(), [snapshot(), snapshot()]),
        Err(EvidenceSessionError::DuplicateSource(_))
    ));
    let extra = SourceSnapshot::try_new(
        SourceId::new("test", "other").unwrap(),
        "r1",
        "sha256:extra",
    )
    .unwrap();
    assert!(matches!(
        EvidenceSession::try_new(project(), [snapshot(), extra]),
        Err(EvidenceSessionError::UnexpectedSource(_))
    ));
}
