//! Entity and type-object attributes resolve as properties in the reserved sets.
#![allow(missing_docs)]

use axioval_engine::{
    EvidenceSession, PropertyRequest, PropertyResolution, PropertyResolutionError,
    PropertyResolutionServiceHandle,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{ATTRIBUTE_SET, ObjectId, PropertyValue, SourceId, TYPE_ATTRIBUTE_SET};

const DATA: &str = "\
#1=IFCSPACE('0000000000000000000001',$,'101',$,$,$,$,'Office',.ELEMENT.,.INTERNAL.,$);
#2=IFCSPACETYPE('0000000000000000000002',$,'OFFICE',$,$,$,$,$,$,.SPACE.,$);
#3=IFCRELDEFINESBYTYPE('0000000000000000000003',$,$,$,(#1),#2);
#4=IFCBUILDINGSTOREY('0000000000000000000004',$,'EG',$,$,$,$,$,.ELEMENT.,3000.);
#5=IFCWALL('0000000000000000000005',$,'W1',$,$,$,$,$,$);
#6=IFCWALLTYPE('0000000000000000000006',$,'WT-A',$,$,$,$,$,$,.STANDARD.);
#7=IFCWALLTYPE('0000000000000000000007',$,'WT-B',$,$,$,$,$,$,.STANDARD.);
#8=IFCRELDEFINESBYTYPE('0000000000000000000008',$,$,$,(#5),#6);
#9=IFCRELDEFINESBYTYPE('0000000000000000000009',$,$,$,(#5),#7);
#10=IFCSPACE('000000000000000000000A',$,$,$,$,$,$,$,.ELEMENT.,$,$);
";

fn session() -> EvidenceSession {
    let bytes = format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n{DATA}ENDSEC;\nEND-ISO-10303-21;\n"
    );
    import_ifc_session("model.ifc", bytes.as_bytes()).unwrap()
}

fn resolve(
    session: &EvidenceSession,
    local: &str,
    set: &str,
    name: &str,
) -> Result<PropertyResolution, PropertyResolutionError> {
    let object = ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), local).unwrap();
    let request = PropertyRequest::try_new(object, Some(set.into()), name).unwrap();
    session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap()
        .resolve(&request)
}

fn value(session: &EvidenceSession, local: &str, set: &str, name: &str) -> Option<PropertyValue> {
    match resolve(session, local, set, name).unwrap() {
        PropertyResolution::Present(resolved) => Some(resolved.property().value.clone()),
        PropertyResolution::Absent(_) => None,
    }
}

#[allow(clippy::unnecessary_wraps)]
fn text(value: &str) -> Option<PropertyValue> {
    Some(PropertyValue::String(value.into()))
}

#[test]
fn a_space_answers_its_number_name_and_enumerations() {
    let session = session();
    assert_eq!(value(&session, "#1", ATTRIBUTE_SET, "Name"), text("101"));
    assert_eq!(
        value(&session, "#1", ATTRIBUTE_SET, "LongName"),
        text("Office")
    );
    // Schema names are matched without regard to case, as EXPRESS does.
    assert_eq!(
        value(&session, "#1", ATTRIBUTE_SET, "longname"),
        text("Office")
    );
    assert_eq!(
        value(&session, "#1", ATTRIBUTE_SET, "PredefinedType"),
        text("INTERNAL")
    );
}

#[test]
fn unset_and_undeclared_attributes_are_exact_absences() {
    let session = session();
    assert_eq!(value(&session, "#10", ATTRIBUTE_SET, "Name"), None);
    assert_eq!(value(&session, "#10", ATTRIBUTE_SET, "LongName"), None);
    // An IfcWall declares no LongName, so it cannot have one.
    assert_eq!(value(&session, "#5", ATTRIBUTE_SET, "LongName"), None);
    let PropertyResolution::Absent(proof) =
        resolve(&session, "#10", ATTRIBUTE_SET, "Name").unwrap()
    else {
        panic!("expected absence");
    };
    assert!(
        proof
            .evidence()
            .locator
            .ends_with("absence:#10:axioval:attributes:Name"),
        "{}",
        proof.evidence().locator
    );
}

#[test]
fn the_type_object_answers_the_construction_type() {
    let session = session();
    assert_eq!(
        value(&session, "#1", TYPE_ATTRIBUTE_SET, "Name"),
        text("OFFICE")
    );
    let PropertyResolution::Present(resolved) =
        resolve(&session, "#1", TYPE_ATTRIBUTE_SET, "Name").unwrap()
    else {
        panic!("expected a type name");
    };
    let locator = &resolved.property().evidence.as_ref().unwrap().locator;
    assert!(locator.ends_with("type-attribute:#3:#2:Name"), "{locator}");
    // No type assigned: exactly absent, not unknown.
    assert_eq!(value(&session, "#10", TYPE_ATTRIBUTE_SET, "Name"), None);
}

#[test]
fn two_type_objects_are_a_conflict_not_a_choice() {
    let session = session();
    assert!(matches!(
        resolve(&session, "#5", TYPE_ATTRIBUTE_SET, "Name"),
        Err(PropertyResolutionError::Conflicting(message)) if message.contains("#6, #7")
    ));
}

#[test]
fn a_measure_without_a_project_unit_is_refused_not_read_bare() {
    // The fixture states no IfcProject, so 3000. has no unit to convert from.
    let session = session();
    assert!(matches!(
        resolve(&session, "#4", ATTRIBUTE_SET, "Elevation"),
        Err(PropertyResolutionError::Incomplete(message)) if message.contains("IFCLENGTHMEASURE")
    ));
}

#[test]
fn property_sets_named_like_attributes_are_untouched() {
    // A set-less request still searches property sets only.
    let session = session();
    let object = ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), "#1").unwrap();
    let request = PropertyRequest::try_new(object, None, "LongName").unwrap();
    assert!(matches!(
        session
            .service::<PropertyResolutionServiceHandle>()
            .unwrap()
            .resolve(&request),
        Ok(PropertyResolution::Absent(_))
    ));
}
