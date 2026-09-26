//! Instances that omit a required relationship end: refused by default,
//! skipped only on request, and always reported by the integrity scan.
//!
//! The fixture mirrors real exports: a second-level space boundary with
//! `RelatedBuildingElement` unset (`$`) for a virtual boundary. IFC4 declares
//! that attribute required; IFC2X3 declared it optional, and exporters kept
//! writing it unset.
#![allow(missing_docs)]

use axioval_engine::{
    AbsentEndPolicy, EvidenceSession, IntegritySeverity, RelationshipQuery,
    RelationshipSelectionError, RelationshipSelectionRequest, RelationshipSelectionServiceHandle,
    SemanticRelationship, SourceIntegrityServiceHandle, TraversalDirection,
};
use axioval_ifc::{
    ABSENT_REQUIRED_END, CONTAINED_TWICE, MALFORMED_RELATIONSHIP, ZONE_MEMBER_NOT_SPATIAL,
    import_ifc_session,
};
use axioval_ir::{ObjectId, SourceId};

fn step(data: &str) -> Vec<u8> {
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
    .into_bytes()
}

/// Space 1 bounded physically by wall 2 (#10) and virtually by nothing (#11).
const BOUNDARIES: &str = "\
#1=IFCSPACE('000000000000000000000s',$,'R1',$,$,$,$,$,$,$,$);
#2=IFCWALL('000000000000000000000w',$,$,$,$,$,$,$,$);
#10=IFCRELSPACEBOUNDARY2NDLEVEL('00000000000000000000b1',$,$,$,#1,#2,$,.PHYSICAL.,.EXTERNAL.,$,$);
#11=IFCRELSPACEBOUNDARY2NDLEVEL('00000000000000000000b2',$,$,$,#1,$,$,.VIRTUAL.,.INTERNAL.,$,$);
";

fn session(data: &str) -> EvidenceSession {
    import_ifc_session("model.ifc", &step(data)).unwrap()
}

fn source() -> SourceId {
    SourceId::new("ifc-step", "model.ifc").unwrap()
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), local).unwrap()
}

fn request(policy: AbsentEndPolicy) -> RelationshipSelectionRequest {
    RelationshipSelectionRequest::try_new(
        id("#1"),
        vec![id("#1"), id("#2")],
        RelationshipQuery::Related {
            relationship: SemanticRelationship::try_new("IfcRelSpaceBoundary").unwrap(),
            direction: TraversalDirection::Forward,
            follow_chain: false,
        },
    )
    .unwrap()
    .with_absent_ends(policy)
}

fn relationships(session: &EvidenceSession) -> &RelationshipSelectionServiceHandle {
    session
        .service::<RelationshipSelectionServiceHandle>()
        .unwrap()
}

#[test]
fn an_absent_required_end_refuses_by_default() {
    let session = session(BOUNDARIES);
    let error = relationships(&session)
        .select(&request(AbsentEndPolicy::default()))
        .unwrap_err();
    let RelationshipSelectionError::Unavailable(message) = error else {
        panic!("expected Unavailable, got {error:?}");
    };
    assert!(message.contains("#11"), "{message}");
    assert!(
        message.contains("skip"),
        "the refusal names the opt-in: {message}"
    );
}

#[test]
fn skipping_answers_from_existing_edges_and_cites_each_skipped_instance() {
    let session = session(BOUNDARIES);
    let selection = relationships(&session)
        .select(&request(AbsentEndPolicy::Skip))
        .unwrap();
    assert_eq!(selection.candidates(), &[id("#2")]);
    let locators: Vec<&str> = selection
        .evidence()
        .iter()
        .map(|item| item.locator.as_str())
        .collect();
    assert!(
        locators
            .iter()
            .any(|locator| locator.ends_with("relationship-absent-end:#11:RelatedBuildingElement")),
        "{locators:?}"
    );
    // The scan locator counts every instance, skipped ones included.
    assert!(
        locators
            .iter()
            .any(|locator| locator.ends_with("relationship-scan:IfcRelSpaceBoundary:2")),
        "{locators:?}"
    );
}

#[test]
fn skipping_does_not_excuse_a_dangling_reference() {
    let data = format!(
        "{BOUNDARIES}#12=IFCRELSPACEBOUNDARY2NDLEVEL('b3',$,$,$,#1,#99,$,.PHYSICAL.,.EXTERNAL.,$,$);\n"
    );
    let session = session(&data);
    assert!(matches!(
        relationships(&session).select(&request(AbsentEndPolicy::Skip)),
        Err(RelationshipSelectionError::Unavailable(message)) if message.contains("#12")
    ));
}

#[test]
fn the_integrity_scan_warns_about_absent_ends_whatever_a_rule_chooses() {
    let session = session(BOUNDARIES);
    let issues = session
        .service::<SourceIntegrityServiceHandle>()
        .expect("the IFC session registers an integrity scan")
        .issues(&source())
        .unwrap();
    assert_eq!(issues.len(), 1, "{issues:?}");
    let issue = &issues[0];
    assert_eq!(issue.code, ABSENT_REQUIRED_END);
    assert_eq!(issue.severity, IntegritySeverity::Warning);
    assert!(
        issue.message.contains("IfcRelSpaceBoundary2ndLevel"),
        "{}",
        issue.message
    );
    assert!(issue.evidence.exact);
    assert!(
        issue
            .evidence
            .locator
            .ends_with("relationship-absent-end:#11:RelatedBuildingElement")
    );
}

#[test]
fn the_integrity_scan_reports_corruption_as_an_error() {
    let data = "\
#1=IFCBUILDINGSTOREY('000000000000000000000s',$,'EG',$,$,$,$,$,$,$);
#2=IFCRELCONTAINEDINSPATIALSTRUCTURE('000000000000000000000c',$,$,$,(#1),#99);
";
    let session = session(data);
    let issues = session
        .service::<SourceIntegrityServiceHandle>()
        .unwrap()
        .issues(&source())
        .unwrap();
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(issues[0].code, MALFORMED_RELATIONSHIP);
    assert_eq!(issues[0].severity, IntegritySeverity::Error);
}

#[test]
fn a_clean_file_has_no_integrity_issues() {
    let data = "\
#1=IFCBUILDINGSTOREY('000000000000000000000s',$,'EG',$,$,$,$,$,$,$);
#2=IFCWALL('000000000000000000000w',$,$,$,$,$,$,$,$);
#3=IFCRELCONTAINEDINSPATIALSTRUCTURE('000000000000000000000c',$,$,$,(#2),#1);
";
    let session = session(data);
    assert!(
        session
            .service::<SourceIntegrityServiceHandle>()
            .unwrap()
            .issues(&source())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn an_absent_required_relating_end_is_flagged_and_refused_too() {
    // Containment without its structure: `RelatingStructure` is required.
    let data = "\
#1=IFCWALL('000000000000000000000w',$,$,$,$,$,$,$,$);
#2=IFCRELCONTAINEDINSPATIALSTRUCTURE('000000000000000000000c',$,$,$,(#1),$);
";
    let session = session(data);
    let issues = session
        .service::<SourceIntegrityServiceHandle>()
        .unwrap()
        .issues(&source())
        .unwrap();
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(issues[0].code, ABSENT_REQUIRED_END);
    assert!(
        issues[0].message.contains("RelatingStructure"),
        "{}",
        issues[0].message
    );
    let request = RelationshipSelectionRequest::try_new(
        id("#1"),
        vec![id("#1")],
        RelationshipQuery::Related {
            relationship: SemanticRelationship::try_new("IfcRelContainedInSpatialStructure")
                .unwrap(),
            direction: TraversalDirection::Backward,
            follow_chain: false,
        },
    )
    .unwrap();
    assert!(matches!(
        relationships(&session).select(&request),
        Err(RelationshipSelectionError::Unavailable(_))
    ));
}

/// Wall #7 contained by two spaces; zone #8 grouping a space and, against
/// the zone's WR1 rule, the wall.
const SPATIAL_CONFLICTS: &str = "\
#5=IFCSPACE('00000000000000000000r1',$,'R1',$,$,$,$,$,$,$,$);
#6=IFCSPACE('00000000000000000000r2',$,'R2',$,$,$,$,$,$,$,$);
#7=IFCWALL('000000000000000000000w',$,$,$,$,$,$,$,$);
#8=IFCZONE('000000000000000000000z',$,'Z',$,$,$);
#20=IFCRELCONTAINEDINSPATIALSTRUCTURE('00000000000000000000c1',$,$,$,(#7),#5);
#21=IFCRELCONTAINEDINSPATIALSTRUCTURE('00000000000000000000c2',$,$,$,(#7),#6);
#22=IFCRELASSIGNSTOGROUP('000000000000000000000g',$,$,$,(#5,#7),$,#8);
";

#[test]
fn the_integrity_scan_warns_about_schema_cardinality_violations() {
    let session = session(SPATIAL_CONFLICTS);
    let issues = session
        .service::<SourceIntegrityServiceHandle>()
        .unwrap()
        .issues(&source())
        .unwrap();
    let codes: Vec<&str> = issues.iter().map(|issue| issue.code.as_str()).collect();
    assert_eq!(
        codes,
        [CONTAINED_TWICE, ZONE_MEMBER_NOT_SPATIAL],
        "{issues:#?}"
    );
    assert!(
        issues
            .iter()
            .all(|issue| issue.severity == IntegritySeverity::Warning)
    );
    assert!(issues[0].message.contains("#7"), "{}", issues[0].message);
    assert!(issues[1].message.contains("#8"), "{}", issues[1].message);
}

#[test]
fn a_single_home_and_a_valid_zone_raise_nothing() {
    let valid = SPATIAL_CONFLICTS
        .replace(
            "#21=IFCRELCONTAINEDINSPATIALSTRUCTURE('00000000000000000000c2',$,$,$,(#7),#6);\n",
            "",
        )
        .replace("(#5,#7),$,#8", "(#5,#6),$,#8");
    let issues = session(&valid)
        .service::<SourceIntegrityServiceHandle>()
        .unwrap()
        .issues(&source())
        .unwrap();
    assert!(issues.is_empty(), "{issues:#?}");
}
