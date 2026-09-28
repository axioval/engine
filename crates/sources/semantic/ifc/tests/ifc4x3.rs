//! IFC4X3 ADD2 sources through the production session.
//!
//! `ifc-properties` (≥ 0.4.1) resolves IFC4X3 exactly, so an IFC4X3 file
//! opens a session bound to the IFC4X3 table and type system. Entities only
//! IFC4X3 declares (`IfcBridge`, `IfcBuiltElement`) are objects with their
//! own ancestry. Classifications are read with the IFC4X3 table too
//! (`ifc-classification` ≥ 0.2.2), so an IFC4X3-only element is classified.
#![allow(missing_docs)]

use axioval_engine::{
    ClassificationAssignment, ClassificationServiceHandle, EvidenceSession, PropertyRequest,
    PropertyResolution, PropertyResolutionServiceHandle, RelationshipQuery,
    RelationshipSelectionRequest, RelationshipSelectionServiceHandle, SemanticRelationship,
    SourceIntegrityServiceHandle, TraversalDirection, TypeHierarchyServiceHandle,
};
use axioval_ifc::{
    IFC4_TYPE_SYSTEM, IFC4X3_TYPE_SYSTEM, ZONE_MEMBER_NOT_SPATIAL, import_ifc_session,
};
use axioval_ir::{ATTRIBUTE_SET, MATERIAL_SET, ObjectId, PropertyValue, SourceId};

const MODEL: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4X3_ADD2'));
ENDSEC;
DATA;
#1=IFCPROJECT('0000000000000000000001',$,'P',$,$,$,$,$,$);
#2=IFCBRIDGE('0000000000000000000002',$,'Bridge',$,$,$,$,$,$,.ARCHED.);
#3=IFCRELAGGREGATES('0000000000000000000003',$,$,$,#1,(#2));
#4=IFCWALL('0000000000000000000004',$,'W1',$,$,$,$,$,.SOLIDWALL.);
#5=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000005',$,$,$,(#4),#2);
#6=IFCPROPERTYSINGLEVALUE('FireRating',$,IFCLABEL('EI 60'),$);
#7=IFCPROPERTYSET('0000000000000000000007',$,'Pset_WallCommon',$,(#6));
#8=IFCRELDEFINESBYPROPERTIES('0000000000000000000008',$,$,$,(#4),#7);
#9=IFCMATERIAL('Concrete',$,$);
#10=IFCRELASSOCIATESMATERIAL('0000000000000000000010',$,$,$,(#4),#9);
#11=IFCCLASSIFICATION($,$,$,'Uniclass',$,$,$);
#12=IFCCLASSIFICATIONREFERENCE($,'EF_25',$,#11,$,$);
#13=IFCRELASSOCIATESCLASSIFICATION('0000000000000000000013',$,$,$,(#4),#12);
#14=IFCROAD('0000000000000000000014',$,'Road',$,$,$,$,$,$,.NOTDEFINED.);
#15=IFCRELAGGREGATES('0000000000000000000015',$,$,$,#1,(#14));
#16=IFCCLASSIFICATION($,$,$,'CCI',$,'https://example.org/cci',$);
#17=IFCCLASSIFICATIONREFERENCE($,'RC',$,#16,$,$);
#18=IFCCLASSIFICATIONREFERENCE($,'RC.1',$,#17,$,$);
#19=IFCRELASSOCIATESCLASSIFICATION('0000000000000000000019',$,$,$,(#14),#18);
ENDSEC;
END-ISO-10303-21;
";

fn source() -> SourceId {
    SourceId::new("ifc-step", "model.ifc").unwrap()
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

fn session() -> EvidenceSession {
    import_ifc_session("model.ifc", MODEL.as_bytes()).unwrap()
}

fn value(session: &EvidenceSession, local: &str, set: &str, name: &str) -> PropertyValue {
    let resolution = session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap()
        .resolve(&PropertyRequest::try_new(id(local), Some(set.into()), name).unwrap());
    match resolution {
        Ok(PropertyResolution::Present(resolved)) => resolved.property().value().clone(),
        other => panic!("{set}.{name} of {local}: {other:?}"),
    }
}

#[test]
fn an_ifc4x3_source_declares_its_own_release_and_type_system() {
    let session = session();
    let snapshot = session.snapshot(&source()).unwrap();
    assert_eq!(snapshot.schema(), Some("IFC4X3"));
    assert_eq!(snapshot.type_systems(), [IFC4X3_TYPE_SYSTEM.into()]);
    assert_ne!(IFC4X3_TYPE_SYSTEM, IFC4_TYPE_SYSTEM);
    let mut kinds: Vec<String> = session
        .project()
        .objects()
        .map(|object| object.kind().to_owned())
        .collect();
    kinds.sort();
    assert_eq!(kinds, ["IFCBRIDGE", "IFCPROJECT", "IFCROAD", "IFCWALL"]);
    // IfcBuiltElement is IFC4X3's name for IFC4's IfcBuildingElement.
    let hierarchy = session.service::<TypeHierarchyServiceHandle>().unwrap();
    assert!(
        hierarchy
            .is_a(&source(), "IfcWall", "IfcBuiltElement")
            .unwrap()
    );
    assert!(
        hierarchy
            .is_a(&source(), "IfcBridge", "IfcFacility")
            .unwrap()
    );
}

#[test]
fn properties_attributes_and_materials_resolve_exactly() {
    let session = session();
    assert_eq!(
        value(&session, "#4", "Pset_WallCommon", "FireRating"),
        PropertyValue::String("EI 60".into())
    );
    assert_eq!(
        value(&session, "#4", ATTRIBUTE_SET, "PredefinedType"),
        PropertyValue::String("SOLIDWALL".into())
    );
    assert_eq!(
        value(&session, "#2", ATTRIBUTE_SET, "PredefinedType"),
        PropertyValue::String("ARCHED".into())
    );
    assert_eq!(
        value(&session, "#4", MATERIAL_SET, "Name"),
        PropertyValue::String("Concrete".into())
    );
}

#[test]
fn relationships_are_read_with_the_ifc4x3_table() {
    let session = session();
    let request = RelationshipSelectionRequest::try_new(
        id("#4"),
        vec![id("#1"), id("#2"), id("#4")],
        RelationshipQuery::Related {
            relationship: SemanticRelationship::try_new("IfcRelContainedInSpatialStructure")
                .unwrap(),
            direction: TraversalDirection::Backward,
            follow_chain: false,
        },
    )
    .unwrap();
    let selection = session
        .service::<RelationshipSelectionServiceHandle>()
        .unwrap()
        .select(&request)
        .unwrap();
    assert_eq!(selection.candidates(), [id("#2")]);
}

#[test]
fn classifications_are_read_with_the_ifc4x3_table() {
    let session = session();
    let service = session.service::<ClassificationServiceHandle>().unwrap();
    assert_eq!(
        service.classifications(&id("#4")).unwrap(),
        [ClassificationAssignment {
            system: Some("Uniclass".into()),
            codes: vec![Some("EF_25".into())],
        }]
    );
    // IfcRoad exists only in IFC4X3, where it is an IfcDefinitionSelect
    // member; read with the IFC4 table its assignment was a reference error.
    // A system stating its `Specification` (IFC4's `Location`) reads alike,
    // and a code keeps its ancestors, outermost last.
    assert_eq!(
        service.classifications(&id("#14")).unwrap(),
        [ClassificationAssignment {
            system: Some("CCI".into()),
            codes: vec![Some("RC.1".into()), Some("RC".into())],
        }]
    );
    assert_eq!(service.classifications(&id("#2")).unwrap(), []);
}

#[test]
fn zones_are_read_with_the_ifc4x3_table() {
    // `ifc-systems` (≥ 0.2.2) reads IFC4X3 zones by attribute name with the
    // IFC4X3 table, so a zone grouping a wall against its WR1 rule is
    // reported, and one grouping only spaces is not.
    let model = |members: &str| {
        MODEL.replace(
            "ENDSEC;\nEND-ISO-10303-21;",
            &format!(
                "#20=IFCSPACE('0000000000000000000020',$,'R1',$,$,$,$,$,$,$,$);
#21=IFCZONE('0000000000000000000021',$,'Z',$,$,$);
#22=IFCRELASSIGNSTOGROUP('0000000000000000000022',$,$,$,({members}),$,#21);
ENDSEC;\nEND-ISO-10303-21;"
            ),
        )
    };
    let codes = |members: &str| {
        let session = import_ifc_session("model.ifc", model(members).as_bytes()).unwrap();
        session
            .service::<SourceIntegrityServiceHandle>()
            .unwrap()
            .issues(&source())
            .unwrap()
            .into_iter()
            .map(|issue| (issue.code, issue.message))
            .collect::<Vec<_>>()
    };
    let flagged = codes("#20,#4");
    assert_eq!(flagged.len(), 1, "{flagged:?}");
    assert_eq!(flagged[0].0, ZONE_MEMBER_NOT_SPATIAL);
    assert!(flagged[0].1.contains("#4"), "{}", flagged[0].1);
    assert_eq!(codes("#20"), []);
}
