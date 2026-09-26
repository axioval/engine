//! A `GlobalId` becomes an external alias only when a consumer can trust it.
#![allow(missing_docs)]

use axioval_engine::{EvidenceSession, IntegritySeverity, SourceIntegrityServiceHandle};
use axioval_ifc::{DUPLICATE_GLOBAL_ID, IFC_GLOBAL_ID, INVALID_GLOBAL_ID, import_ifc_session};
use axioval_ir::{ObjectId, SourceId};

const WALL_ID: &str = "2O2Fr$t4X7Zf8NOew3FLOH";
const SPACE_ID: &str = "0000000000000000000000";

fn session(schema: &str, data: &str) -> EvidenceSession {
    let bytes = format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('{schema}'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
    );
    import_ifc_session("model.ifc", bytes.as_bytes()).unwrap()
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), local).unwrap()
}

fn global_id<'a>(session: &'a EvidenceSession, local: &str) -> Option<&'a str> {
    session
        .project()
        .object(&id(local))
        .expect("object is in the project")
        .external_id(IFC_GLOBAL_ID)
}

fn issues(session: &EvidenceSession) -> Vec<(String, IntegritySeverity, String)> {
    session
        .service::<SourceIntegrityServiceHandle>()
        .unwrap()
        .issues(&SourceId::new("ifc-step", "model.ifc").unwrap())
        .unwrap()
        .into_iter()
        .map(|issue| (issue.code, issue.severity, issue.message))
        .collect()
}

#[test]
fn valid_unique_global_ids_are_aliases_beside_the_step_identity() {
    for (schema, wall) in [
        ("IFC4", "IFCWALL('{W}',$,$,$,$,$,$,$,$)"),
        ("IFC2X3", "IFCWALL('{W}',$,$,$,$,$,$,$)"),
    ] {
        let data = format!(
            "#1={};\n#2=IFCBUILDINGSTOREY('{SPACE_ID}',$,'EG',$,$,$,$,$,$,$);\n",
            wall.replace("{W}", WALL_ID)
        );
        let session = session(schema, &data);
        // The engine keeps keying on the STEP instance; the alias sits beside it.
        assert_eq!(global_id(&session, "#1"), Some(WALL_ID), "{schema}");
        assert_eq!(global_id(&session, "#2"), Some(SPACE_ID), "{schema}");
        assert!(
            issues(&session).is_empty(),
            "{schema}: {:?}",
            issues(&session)
        );
    }
}

#[test]
fn a_malformed_global_id_leaves_the_object_checkable_without_an_alias() {
    let cases = [
        ("'w'", "too short"),
        ("$", "unset"),
        ("'0123456789ABCDEFGHIJ+/'", "foreign alphabet"),
        // Accepted by `Guid::parse` alone, but its top bits do not fit a
        // UUID, so it would collide with '0000000000000000000000'.
        ("'4000000000000000000000'", "out-of-range leading digit"),
    ];
    for (value, why) in cases {
        let session = session("IFC4", &format!("#1=IFCWALL({value},$,$,$,$,$,$,$,$);\n"));
        assert_eq!(global_id(&session, "#1"), None, "{why}");
        let issues = issues(&session);
        assert_eq!(issues.len(), 1, "{why}: {issues:?}");
        let (code, severity, message) = &issues[0];
        assert_eq!(code, INVALID_GLOBAL_ID, "{why}");
        assert_eq!(*severity, IntegritySeverity::Warning, "{why}");
        assert!(message.contains("#1"), "{why}: {message}");
    }
}

#[test]
fn a_shared_global_id_is_attached_to_none_of_its_claimants() {
    // The relationship is not a project object, but a viewer resolving the
    // id could still land on it, so it makes the wall's id ambiguous.
    let data = format!(
        "#1=IFCWALL('{WALL_ID}',$,$,$,$,$,$,$,$);\n\
         #2=IFCWALL('{WALL_ID}',$,$,$,$,$,$,$,$);\n\
         #3=IFCBUILDINGSTOREY('{SPACE_ID}',$,'EG',$,$,$,$,$,$,$);\n\
         #4=IFCRELCONTAINEDINSPATIALSTRUCTURE('{SPACE_ID}',$,$,$,(#1),#3);\n"
    );
    let session = session("IFC4", &data);
    for local in ["#1", "#2", "#3"] {
        assert_eq!(global_id(&session, local), None, "{local}");
    }
    let issues = issues(&session);
    let codes: Vec<&str> = issues.iter().map(|(code, ..)| code.as_str()).collect();
    assert_eq!(
        codes,
        [DUPLICATE_GLOBAL_ID, DUPLICATE_GLOBAL_ID],
        "{issues:?}"
    );
    // Ordered by GlobalId; each names every claimant in file order.
    assert!(issues[0].2.contains("#3, #4"), "{}", issues[0].2);
    assert!(issues[1].2.contains("#1, #2"), "{}", issues[1].2);
}
