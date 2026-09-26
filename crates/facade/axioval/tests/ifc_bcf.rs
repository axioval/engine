//! End to end: IFC GlobalIds through the IFC adapter into BCF viewpoints.
//!
//! Lives in the facade because it spans the IFC adapter and the BCF sink,
//! which must not depend on each other: a dev-dependency between unpublished
//! siblings breaks workspace package verification.
#![cfg(all(feature = "ifc", feature = "bcf"))]
#![allow(missing_docs, clippy::doc_markdown)]

use axioval::bcf::{IFC_GLOBAL_ID_SCHEME, Options, export};
use axioval::ifc::{IFC_GLOBAL_ID, import_ifc_session};
use axioval::ir::{Finding, ObjectId, Report, RuleId, Severity, SourceId};
use openbim_bcf::Component;

const WALL: &str = "2O2Fr$t4X7Zf8NOew3FLOH";
const SLAB: &str = "0000000000000000000001";

#[test]
fn the_sink_reads_the_scheme_the_adapter_writes() {
    assert_eq!(IFC_GLOBAL_ID_SCHEME, IFC_GLOBAL_ID);
}

#[test]
fn an_ifc_finding_selects_its_elements_by_global_id() {
    let step = format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCWALL('{WALL}',$,$,$,$,$,$,$,$);\n\
         #2=IFCSLAB('{SLAB}',$,$,$,$,$,$,$,$);\n\
         #3=IFCDOOR('bad',$,$,$,$,$,$,$,$,$,$,$,$);\n\
         ENDSEC;\nEND-ISO-10303-21;\n"
    );
    let session = import_ifc_session("model.ifc", step.as_bytes()).unwrap();
    let id = |local: &str| {
        ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), local).unwrap()
    };
    let finding = |object: &str, related: &str| {
        Finding {
            rule_id: RuleId::new("contact").unwrap(),
            object_id: id(object),
            severity: Severity::Error,
            message: format!("{object} has insufficient contact"),
            related: vec![],
            evidence: vec![],
        }
        .with_related([id(related)])
    };
    let report = Report {
        findings: vec![finding("#1", "#2"), finding("#3", "#1")],
        not_evaluated: vec![],
    };

    let export = export(
        &report,
        session.project(),
        &Options::new("axioval", "2026-09-26T10:00:00Z"),
    )
    .unwrap();
    let topics = &export.document.topics;
    assert_eq!(
        topics[0].viewpoints[0].selection,
        [Component::ifc(WALL), Component::ifc(SLAB)]
    );
    // The door's GlobalId is malformed, so the adapter attached no alias and
    // the sink says so instead of selecting only the wall.
    assert!(topics[1].viewpoints.is_empty());
    assert_eq!(export.unanchored, [id("#3")]);
    assert!(!export.to_bytes().unwrap().is_empty());
}
