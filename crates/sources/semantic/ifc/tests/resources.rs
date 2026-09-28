//! Resource objects: every instance that is no occurrence, context or type
//! object, listed by class, with their attributes, classifications and
//! material properties.
#![allow(missing_docs)]

use axioval_engine::{
    ClassificationAssignment, ClassificationServiceHandle, EvidenceSession, PropertyRequest,
    PropertyResolution, PropertyResolutionError, PropertyResolutionServiceHandle, ResourceRequest,
    ResourceServiceHandle,
};
use axioval_ifc::{IFC_GLOBAL_ID, import_ifc_session};
use axioval_ir::{ATTRIBUTE_SET, ObjectId, PropertyValue, SourceId};

const IFC4: &str = "\
#1=IFCPROJECT('0000000000000000000001',$,'P',$,$,$,$,$,$);
#2=IFCWALL('0000000000000000000002',$,'W1',$,$,$,$,$,$);
#3=IFCWALL('0000000000000000000003',$,'W2',$,$,$,$,$,$);
#4=IFCRELCONNECTSPATHELEMENTS('0000000000000000000004',$,$,$,$,#2,#3,(),(),.ATSTART.,.ATEND.);
#5=IFCMATERIAL('Concrete',$,$);
#6=IFCMATERIALLAYER(#5,0.2,$,$,$,$,$);
#7=IFCMATERIALLAYERSET((#6),'Build-up',$);
#8=IFCPRESENTATIONLAYERWITHSTYLE('Layer',$,(#9),$,.U.,.F.,.F.,());
#9=IFCCARTESIANPOINT((0.,0.,0.));
#10=IFCCLASSIFICATION($,$,$,'Uniclass',$,$,$);
#11=IFCCLASSIFICATIONREFERENCE($,'Ss_25',$,#10,$,$);
#12=IFCEXTERNALREFERENCERELATIONSHIP($,$,#11,(#5));
#13=IFCSURFACESTYLERENDERING(#14,$,IFCNORMALISEDRATIOMEASURE(0.5),$,$,$,$,$,.FLAT.);
#14=IFCCOLOURRGB($,1.,1.,1.);
#15=IFCSURFACESTYLERENDERING(#14,$,#14,$,$,$,$,$,.FLAT.);
";

fn session(schema: &str, data: &str) -> EvidenceSession {
    let bytes = format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('{schema}'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
    );
    import_ifc_session("model.ifc", bytes.as_bytes()).unwrap()
}

fn source() -> SourceId {
    SourceId::new("ifc-step", "model.ifc").unwrap()
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

fn listed(session: &EvidenceSession, class: &str, subtypes: bool) -> Vec<String> {
    let request = ResourceRequest::try_new(source(), class, subtypes).unwrap();
    session
        .service::<ResourceServiceHandle>()
        .unwrap()
        .resources(&request)
        .unwrap()
        .into_iter()
        .map(|object| format!("{} {}", object.id.local_id, object.kind))
        .collect()
}

fn resolve(
    session: &EvidenceSession,
    local: &str,
    set: Option<&str>,
    name: &str,
) -> Result<PropertyResolution, PropertyResolutionError> {
    let request = PropertyRequest::try_new(id(local), set.map(str::to_owned), name).unwrap();
    session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap()
        .resolve(&request)
}

fn attribute(session: &EvidenceSession, local: &str, name: &str) -> Option<PropertyValue> {
    match resolve(session, local, Some(ATTRIBUTE_SET), name).unwrap() {
        PropertyResolution::Present(resolved) => Some(resolved.property().value.clone()),
        PropertyResolution::Absent(_) => None,
    }
}

#[test]
fn a_resource_class_lists_its_instances_and_an_object_class_none() {
    let session = session("IFC4", IFC4);
    assert!(session.project().object(&id("#5")).is_none());
    assert_eq!(listed(&session, "IfcMaterial", false), ["#5 IFCMATERIAL"]);
    assert_eq!(
        listed(&session, "IfcMaterialDefinition", true),
        [
            "#5 IFCMATERIAL",
            "#6 IFCMATERIALLAYER",
            "#7 IFCMATERIALLAYERSET"
        ]
    );
    // Not the subtypes unless asked.
    assert!(listed(&session, "IfcMaterialDefinition", false).is_empty());
    // A class of objects, or one with object subclasses, selects objects only.
    assert!(listed(&session, "IfcWall", false).is_empty());
    assert!(listed(&session, "IfcRoot", true).is_empty());
    assert!(listed(&session, "IfcNoSuchClass", true).is_empty());
}

#[test]
fn a_rooted_resource_carries_its_global_id() {
    let session = session("IFC4", IFC4);
    let request = ResourceRequest::try_new(source(), "IfcRelConnectsPathElements", false).unwrap();
    let relations = session
        .service::<ResourceServiceHandle>()
        .unwrap()
        .resources(&request)
        .unwrap();
    assert_eq!(relations.len(), 1);
    assert_eq!(
        relations[0].external_id(IFC_GLOBAL_ID),
        Some("0000000000000000000004")
    );
}

#[test]
fn empty_aggregates_and_logical_unknowns_state_no_value() {
    let session = session("IFC4", IFC4);
    assert_eq!(attribute(&session, "#4", "RelatingPriorities"), None);
    assert_eq!(attribute(&session, "#8", "LayerStyles"), None);
    assert_eq!(attribute(&session, "#8", "LayerOn"), None);
    assert_eq!(
        attribute(&session, "#8", "LayerBlocked"),
        Some(PropertyValue::Boolean(false))
    );
    // An aggregate stating members is still no scalar.
    assert!(matches!(
        resolve(&session, "#8", Some(ATTRIBUTE_SET), "AssignedItems"),
        Err(PropertyResolutionError::Unavailable(_))
    ));
}

#[test]
fn a_reference_names_the_instance_and_a_typed_select_its_value() {
    let session = session("IFC4", IFC4);
    assert_eq!(
        attribute(&session, "#4", "RelatingElement"),
        Some(PropertyValue::Reference(id("#2")))
    );
    assert_eq!(
        attribute(&session, "#15", "DiffuseColour"),
        Some(PropertyValue::Reference(id("#14")))
    );
    assert_eq!(
        attribute(&session, "#13", "DiffuseColour"),
        Some(PropertyValue::Decimal(0.5))
    );
}

fn classifications(session: &EvidenceSession, local: &str) -> Vec<ClassificationAssignment> {
    session
        .service::<ClassificationServiceHandle>()
        .unwrap()
        .classifications(&id(local))
        .unwrap()
}

#[test]
fn a_material_is_classified_through_an_external_reference_relationship() {
    let session = session("IFC4", IFC4);
    assert_eq!(
        classifications(&session, "#5"),
        [ClassificationAssignment {
            system: Some("Uniclass".into()),
            codes: vec![Some("Ss_25".into())],
        }]
    );
    assert!(classifications(&session, "#7").is_empty());
    assert!(classifications(&session, "#2").is_empty());
}

#[test]
fn an_ifc2x3_material_is_classified_through_its_classification_relationship() {
    let session = session(
        "IFC2X3",
        "\
#1=IFCMATERIAL('Steel');
#2=IFCCLASSIFICATION('src','1',$,'Uniclass');
#3=IFCCLASSIFICATIONREFERENCE($,'Pr_20',$,#2);
#4=IFCMATERIALCLASSIFICATIONRELATIONSHIP((#3),#1);
",
    );
    assert_eq!(
        classifications(&session, "#1"),
        [ClassificationAssignment {
            system: Some("Uniclass".into()),
            codes: vec![Some("Pr_20".into())],
        }]
    );
}

#[test]
fn a_material_carries_no_property_only_in_a_model_without_material_properties() {
    let without = session("IFC4", IFC4);
    assert!(matches!(
        resolve(&without, "#5", Some("Custom_Pset"), "Foo"),
        Ok(PropertyResolution::Absent(_))
    ));
    // The property library reads no material property set: refused.
    let with = session(
        "IFC4",
        "\
#1=IFCMATERIAL('Concrete',$,$);
#2=IFCPROPERTYSINGLEVALUE('Foo',$,IFCLABEL('Bar'),$);
#3=IFCMATERIALPROPERTIES('Custom_Pset',$,(#2),#1);
",
    );
    assert!(matches!(
        resolve(&with, "#1", Some("Custom_Pset"), "Foo"),
        Err(PropertyResolutionError::Unavailable(_))
    ));
    // Other resources stay refused, never read as absent.
    assert!(matches!(
        resolve(&without, "#8", Some("Custom_Pset"), "Foo"),
        Err(PropertyResolutionError::Unavailable(_))
    ));
}
