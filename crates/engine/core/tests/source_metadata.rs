//! Source metadata: what a session states about each source as a whole.
#![allow(missing_docs)]

use axioval_engine::{EvidenceSession, EvidenceSessionError, SourceMetadata, SourceSnapshot};
use axioval_ir::contract::SourceField;
use axioval_ir::{Object, ObjectId, Project, SourceId};

fn source(document: &str) -> SourceId {
    SourceId::new("test", document).unwrap()
}

fn session(document: &str) -> EvidenceSession {
    let object = Object::new(ObjectId::new(source(document), "42").unwrap(), "WALL");
    EvidenceSession::try_new(
        Project::new(vec![object]).unwrap(),
        [SourceSnapshot::try_new(source(document), "r", "f").unwrap()],
    )
    .unwrap()
}

fn one(value: &str) -> Vec<String> {
    vec![value.to_owned()]
}

#[test]
fn source_metadata_adds_up_and_refuses_contradictions() {
    let session = session("a")
        .with_source_metadata(
            &source("a"),
            SourceMetadata::new().with(SourceField::Application, ["Tool"]),
        )
        .unwrap()
        .with_source_metadata(
            &source("a"),
            SourceMetadata::new().with(SourceField::FileName, ["a.ifc"]),
        )
        .unwrap();
    let metadata = session.source_metadata(&source("a")).unwrap();
    assert_eq!(
        metadata
            .values(SourceField::Application)
            .map(<[String]>::to_vec),
        Some(one("Tool"))
    );
    assert_eq!(
        metadata
            .values(SourceField::FileName)
            .map(<[String]>::to_vec),
        Some(one("a.ifc"))
    );
    assert_eq!(
        session
            .with_source_metadata(
                &source("a"),
                SourceMetadata::new().with(SourceField::Application, ["Other"]),
            )
            .err(),
        Some(EvidenceSessionError::ConflictingMetadata(
            source("a"),
            "application"
        ))
    );
}

#[test]
fn metadata_for_a_source_outside_the_session_is_refused() {
    assert_eq!(
        session("a")
            .with_source_metadata(&source("b"), SourceMetadata::new())
            .err(),
        Some(EvidenceSessionError::UnknownSource(source("b")))
    );
}

#[test]
fn federation_keeps_every_members_metadata() {
    let member = |document: &str, application: &str| {
        session(document)
            .with_source_metadata(
                &source(document),
                SourceMetadata::new().with(SourceField::Application, [application]),
            )
            .unwrap()
    };
    let federated =
        EvidenceSession::federate([member("a", "Tool A"), member("b", "Tool B")]).unwrap();
    for (document, application) in [("a", "Tool A"), ("b", "Tool B")] {
        assert_eq!(
            federated
                .source_metadata(&source(document))
                .unwrap()
                .values(SourceField::Application)
                .map(<[String]>::to_vec),
            Some(one(application))
        );
    }
}
