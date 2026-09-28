//! Resource objects: every instance that is no occurrence, context or type
//! object, listed by class, with their attributes, classifications and
//! material properties.
#![allow(missing_docs)]

use axioval_engine::{
    ClassificationAssignment, ClassificationServiceHandle, EvidenceSession, NameMatch,
    PropertyEnumeration, PropertyEnumerationRequest, PropertyRequest, PropertyResolution,
    PropertyResolutionError, PropertyResolutionServiceHandle, ResourceRequest,
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

/// IFC4 material definitions: material #1 with a `Custom_Pset` and an
/// empty `Empty_Pset`, material #4 with none, and layer #5 with an unnamed
/// set of its own; #7 is a resource that is no material.
const IFC4_MATERIALS: &str = "\
#1=IFCMATERIAL('Concrete',$,$);
#2=IFCPROPERTYSINGLEVALUE('Foo',$,IFCLABEL('Bar'),$);
#3=IFCMATERIALPROPERTIES('Custom_Pset',$,(#2),#1);
#4=IFCMATERIAL('Steel',$,$);
#5=IFCMATERIALLAYER(#4,0.2,$,$,$,$,$);
#6=IFCMATERIALPROPERTIES($,$,(#8),#5);
#7=IFCCARTESIANPOINT((0.,0.,0.));
#8=IFCPROPERTYSINGLEVALUE('Grade',$,IFCLABEL('S355'),$);
#9=IFCMATERIALPROPERTIES('Empty_Pset',$,(),#1);
";

fn label(resolution: Result<PropertyResolution, PropertyResolutionError>) -> (String, String) {
    let Ok(PropertyResolution::Present(resolved)) = resolution else {
        panic!("present: {resolution:?}");
    };
    let property = resolved.property();
    let PropertyValue::String(text) = &property.value else {
        panic!("a label: {property:?}");
    };
    (text.clone(), property.data_type().unwrap().to_owned())
}

fn enumerate(
    session: &EvidenceSession,
    local: &str,
    set: NameMatch,
) -> Result<PropertyEnumeration, PropertyResolutionError> {
    let request = PropertyEnumerationRequest::try_new(id(local), set, NameMatch::Any).unwrap();
    session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap()
        .enumerate(&request)
}

#[test]
fn a_material_definition_carries_its_own_property_sets() {
    let materials = session("IFC4", IFC4_MATERIALS);
    assert_eq!(
        label(resolve(&materials, "#1", Some("Custom_Pset"), "Foo")),
        ("Bar".to_owned(), "IFCLABEL".to_owned())
    );
    let Ok(PropertyResolution::Present(resolved)) =
        resolve(&materials, "#1", Some("Custom_Pset"), "Foo")
    else {
        panic!("Custom_Pset.Foo is present");
    };
    assert!(
        resolved
            .property()
            .evidence
            .as_ref()
            .unwrap()
            .locator
            .ends_with(":material:#1:#3/#2"),
        "{resolved:?}"
    );
    // A set without a name is keyed by its entity; a layer's sets are its
    // own, never its material's.
    assert_eq!(
        label(resolve(
            &materials,
            "#5",
            Some("IfcMaterialProperties"),
            "Grade"
        ))
        .0,
        "S355"
    );
    // Proven absences: another name, another set, a material without sets,
    // a set holding nothing.
    for (local, set, name) in [
        ("#1", Some("Custom_Pset"), "Baz"),
        ("#1", Some("Other_Pset"), "Foo"),
        ("#4", Some("Custom_Pset"), "Foo"),
        ("#1", Some("Empty_Pset"), "Foo"),
    ] {
        assert!(
            matches!(
                resolve(&materials, local, set, name),
                Ok(PropertyResolution::Absent(_))
            ),
            "{local} {set:?}.{name}"
        );
    }
    let listed = enumerate(&materials, "#1", NameMatch::Any).unwrap();
    assert_eq!(listed.properties().len(), 1);
    assert_eq!(listed.properties()[0].name, "Foo");
    assert_eq!(listed.empty_sets(), ["Empty_Pset"]);
    assert!(
        enumerate(&materials, "#4", NameMatch::Any)
            .unwrap()
            .properties()
            .is_empty()
    );
    // A resource that is no material has no properties to read: refused,
    // never absent.
    assert!(matches!(
        resolve(&materials, "#7", Some("Custom_Pset"), "Foo"),
        Err(PropertyResolutionError::Unavailable(_))
    ));
    assert!(matches!(
        enumerate(&materials, "#7", NameMatch::Any),
        Err(PropertyResolutionError::Unavailable(_))
    ));
    let without = session("IFC4", IFC4);
    assert!(matches!(
        resolve(&without, "#5", Some("Custom_Pset"), "Foo"),
        Ok(PropertyResolution::Absent(_))
    ));
}

/// IFC2X3: extended material properties by name, typed ones by entity; a
/// layer is no material there.
#[test]
fn an_ifc2x3_material_carries_its_extended_and_typed_properties() {
    let materials = session(
        "IFC2X3",
        "\
#1=IFCMATERIAL('Concrete');
#2=IFCPROPERTYSINGLEVALUE('Foo',$,IFCLABEL('Bar'),$);
#3=IFCEXTENDEDMATERIALPROPERTIES(#1,(#2),$,'Custom_Pset');
#4=IFCMECHANICALMATERIALPROPERTIES(#1,$,$,$,0.2,$);
#5=IFCMATERIALLAYER(#1,0.2,$);
",
    );
    assert_eq!(
        label(resolve(&materials, "#1", Some("Custom_Pset"), "Foo")),
        ("Bar".to_owned(), "IFCLABEL".to_owned())
    );
    // A typed set is a predefined set named by its entity: an unset
    // attribute is present, null, with its declared type.
    let Ok(PropertyResolution::Present(viscosity)) = resolve(
        &materials,
        "#1",
        Some("IfcMechanicalMaterialProperties"),
        "DynamicViscosity",
    ) else {
        panic!("DynamicViscosity is present");
    };
    assert_eq!(viscosity.property().value, PropertyValue::Null);
    assert_eq!(
        viscosity.property().data_type(),
        Some("IFCDYNAMICVISCOSITYMEASURE")
    );
    // `Material` names the owner, never a member.
    assert!(matches!(
        resolve(
            &materials,
            "#1",
            Some("IfcMechanicalMaterialProperties"),
            "Material"
        ),
        Ok(PropertyResolution::Absent(_))
    ));
    assert!(matches!(
        resolve(&materials, "#5", Some("Custom_Pset"), "Foo"),
        Err(PropertyResolutionError::Unavailable(_))
    ));
}
