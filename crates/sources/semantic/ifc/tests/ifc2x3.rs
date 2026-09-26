//! IFC2X3 sources through the production session.
//!
//! These tests pin the places where IFC2X3 and IFC4 disagree. Each one would
//! pass or fail differently if the adapter quietly fell back to IFC4 tables,
//! which is the regression they exist to catch.
#![allow(missing_docs)]

use axioval_engine::{
    AbsentEndPolicy, EvidenceSession, PropertyRequest, PropertyResolution,
    PropertyResolutionServiceHandle, RelationshipQuery, RelationshipSelectionRequest,
    RelationshipSelectionServiceHandle, SemanticRelationship, SourceIntegrityServiceHandle,
    TraversalDirection, TypeHierarchyServiceHandle,
};
use axioval_ifc::{IFC2X3_TYPE_SYSTEM, IFC4_TYPE_SYSTEM, import_ifc_session};
use axioval_ir::{ObjectId, PropertyValue, SourceId};

fn step(schema: &str, data: &str) -> Vec<u8> {
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('{schema}'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
    .into_bytes()
}

fn source() -> SourceId {
    SourceId::new("ifc-step", "model.ifc").unwrap()
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

fn session(schema: &str, data: &str) -> EvidenceSession {
    import_ifc_session("model.ifc", &step(schema, data)).unwrap()
}

/// A wall with a property through the IFC2X3 shape of
/// `IfcRelDefinesByProperties` (relating end is a plain property set), a
/// space with a virtual boundary that names no element, and a door typed by
/// an `IfcDoorStyle`, which IFC2X3 has and IFC4 renamed.
const BUILDING: &str = "\
#1=IFCSPACE('000000000000000000000s',$,'R1',$,$,$,$,$,.ELEMENT.,.INTERNAL.,$);
#2=IFCWALL('000000000000000000000w',$,$,$,$,$,$,$);
#3=IFCDOOR('000000000000000000000d',$,$,$,$,$,$,$,$,$);
#4=IFCDOORSTYLE('00000000000000000000ds',$,'Style',$,$,$,$,$,.SINGLE_SWING_LEFT.,.WOOD.,.F.,.F.);
#5=IFCRELDEFINESBYTYPE('000000000000000000000t',$,$,$,(#3),#4);
#6=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('W-1'),$);
#7=IFCPROPERTYSET('000000000000000000000p',$,'Pset_WallCommon',$,(#6));
#8=IFCRELDEFINESBYPROPERTIES('000000000000000000000r',$,$,$,(#2),#7);
#10=IFCRELSPACEBOUNDARY('00000000000000000000b1',$,$,$,#1,#2,$,.PHYSICAL.,.EXTERNAL.);
#11=IFCRELSPACEBOUNDARY('00000000000000000000b2',$,$,$,#1,$,$,.VIRTUAL.,.INTERNAL.);
";

#[test]
fn an_ifc2x3_source_declares_its_own_release_and_type_system() {
    let session = session("IFC2X3", BUILDING);
    let snapshot = session.snapshot(&source()).unwrap();
    assert_eq!(snapshot.schema(), Some("IFC2X3"));
    assert_eq!(snapshot.type_systems(), [IFC2X3_TYPE_SYSTEM.into()]);
    // The two releases never share a type system, so a package bound to
    // IFC4 names cannot bind IFC2X3 data by accident.
    assert_ne!(IFC2X3_TYPE_SYSTEM, IFC4_TYPE_SYSTEM);
}

#[test]
fn objects_are_discovered_with_the_declared_release_ancestry() {
    // IfcElectricalCircuit exists only in IFC2X3, where it is an IfcObject.
    // IfcProject is an IfcObject in IFC2X3 but an IfcContext in IFC4. Read
    // with IFC4 ancestry, both would drop out of an IFC2X3 project.
    let data = "\
#1=IFCPROJECT('000000000000000000000p',$,'P',$,$,$,$,$,$);
#2=IFCELECTRICALCIRCUIT('000000000000000000000c',$,'Circuit',$,$);
#3=IFCWALL('000000000000000000000w',$,$,$,$,$,$,$);
";
    let x3 = session("IFC2X3", data);
    let kinds = |session: &EvidenceSession| {
        let mut kinds: Vec<String> = session
            .project()
            .objects()
            .map(|object| object.kind().to_ascii_uppercase())
            .collect();
        kinds.sort();
        kinds
    };
    assert_eq!(
        kinds(&x3),
        ["IFCELECTRICALCIRCUIT", "IFCPROJECT", "IFCWALL"]
    );
    // The same project under IFC4 ancestry is a context, not an object.
    let x4 = session(
        "IFC4",
        "#1=IFCPROJECT('p',$,'P',$,$,$,$,$,$);\n#3=IFCWALL('w',$,$,$,$,$,$,$,$);\n",
    );
    assert_eq!(kinds(&x4), ["IFCWALL"]);
}

#[test]
fn properties_resolve_through_the_ifc2x3_relationship_shape() {
    let session = session("IFC2X3", BUILDING);
    let properties = session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap();
    let request =
        PropertyRequest::try_new(id("#2"), Some("Pset_WallCommon".into()), "Reference").unwrap();
    let PropertyResolution::Present(resolved) = properties.resolve(&request).unwrap() else {
        panic!("the wall's Reference is present");
    };
    assert_eq!(
        resolved.property().value(),
        &PropertyValue::String("W-1".into())
    );
}

#[test]
fn the_type_hierarchy_is_answered_from_ifc2x3() {
    let session = session("IFC2X3", BUILDING);
    let hierarchy = session.service::<TypeHierarchyServiceHandle>().unwrap();
    // IfcDoorStyle exists only in IFC2X3; IFC4 would call it unknown.
    assert!(
        hierarchy
            .is_a(&source(), "IfcDoorStyle", "IfcTypeObject")
            .unwrap()
    );
    // IfcDoorType exists only in IFC4, so an IFC2X3 source refuses it
    // rather than calling it a non-member.
    assert!(
        hierarchy
            .is_a(&source(), "IfcDoorType", "IfcTypeObject")
            .is_err()
    );
}

#[test]
fn an_unset_ifc2x3_boundary_element_is_legal_not_a_warning() {
    // IFC2X3 declares IfcRelSpaceBoundary.RelatedBuildingElement OPTIONAL.
    // The same `$` that IFC4 flags is valid here, so there is nothing to
    // report and nothing a strict rule needs to refuse.
    let session = session("IFC2X3", BUILDING);
    let issues = session
        .service::<SourceIntegrityServiceHandle>()
        .unwrap()
        .issues(&source())
        .unwrap();
    assert!(issues.is_empty(), "{issues:?}");

    let request = RelationshipSelectionRequest::try_new(
        id("#1"),
        vec![id("#1"), id("#2"), id("#3")],
        RelationshipQuery::Related {
            relationship: SemanticRelationship::try_new("IfcRelSpaceBoundary").unwrap(),
            direction: TraversalDirection::Forward,
            follow_chain: false,
        },
    )
    .unwrap();
    assert_eq!(request.absent_ends(), AbsentEndPolicy::Refuse);
    let selection = session
        .service::<RelationshipSelectionServiceHandle>()
        .unwrap()
        .select(&request)
        .expect("strict policy answers: no required end is absent in IFC2X3");
    assert_eq!(selection.candidates(), [id("#2")].as_slice());
}

#[test]
fn the_same_unset_end_in_ifc4_is_still_a_warning() {
    // The IFC4 counterpart of the fixture above, so the difference is the
    // declared release and nothing else.
    let ifc4 = "\
#1=IFCSPACE('000000000000000000000s',$,'R1',$,$,$,$,$,.ELEMENT.,.INTERNAL.,$,$);
#2=IFCWALL('000000000000000000000w',$,$,$,$,$,$,$,$);
#10=IFCRELSPACEBOUNDARY('00000000000000000000b1',$,$,$,#1,#2,$,.PHYSICAL.,.EXTERNAL.);
#11=IFCRELSPACEBOUNDARY('00000000000000000000b2',$,$,$,#1,$,$,.VIRTUAL.,.INTERNAL.);
";
    let session = session("IFC4", ifc4);
    let issues = session
        .service::<SourceIntegrityServiceHandle>()
        .unwrap()
        .issues(&source())
        .unwrap();
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert!(issues[0].message.contains("IFC4"), "{}", issues[0].message);
}
