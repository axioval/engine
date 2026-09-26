//! Predefined types resolve in the reserved set as IDS reads them: type
//! object first.
#![allow(missing_docs)]

use axioval_engine::{
    EvidenceSession, PropertyRequest, PropertyResolution, PropertyResolutionError,
    PropertyResolutionServiceHandle,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{
    ObjectId, PREDEFINED_TYPE, PREDEFINED_TYPE_SET, PREDEFINED_TYPE_USER_DEFINED, PropertyValue,
    SourceId,
};

fn session(data: &str) -> EvidenceSession {
    let bytes = format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
    );
    import_ifc_session("model.ifc", bytes.as_bytes()).unwrap()
}

fn read(
    session: &EvidenceSession,
    local: &str,
    name: &str,
) -> Result<Option<PropertyValue>, PropertyResolutionError> {
    let id = ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), local).unwrap();
    let request = PropertyRequest::try_new(id, Some(PREDEFINED_TYPE_SET.into()), name).unwrap();
    let resolution = session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap()
        .resolve(&request)?;
    Ok(match resolution {
        PropertyResolution::Present(resolved) => {
            assert!(resolved.property().evidence.iter().all(|e| e.exact));
            Some(resolved.property().value.clone())
        }
        PropertyResolution::Absent(proof) => {
            assert!(proof.evidence().exact);
            None
        }
    })
}

/// The designation and whether it is user-defined.
fn resolve(
    session: &EvidenceSession,
    local: &str,
) -> Result<(Option<String>, bool), PropertyResolutionError> {
    let value = match read(session, local, PREDEFINED_TYPE)? {
        None => None,
        Some(PropertyValue::String(text)) => Some(text),
        Some(other) => panic!("not text: {other:?}"),
    };
    let Some(PropertyValue::Boolean(user_defined)) =
        read(session, local, PREDEFINED_TYPE_USER_DEFINED)?
    else {
        panic!("UserDefined is always a boolean");
    };
    Ok((value, user_defined))
}

#[test]
fn predefined_types_resolve_type_first_then_occurrence() {
    let session = session(
        "#1=IFCWALL('0000000000000000000001',$,$,$,$,$,$,$,.SOLIDWALL.);
#2=IFCWALL('0000000000000000000002',$,$,$,'WALDO',$,$,$,.USERDEFINED.);
#3=IFCWALL('0000000000000000000003',$,$,$,$,$,$,$,$);
#4=IFCWALL('0000000000000000000004',$,$,$,$,$,$,$,$);
#5=IFCWALLTYPE('0000000000000000000005',$,$,$,$,$,$,$,'X',.USERDEFINED.);
#6=IFCRELDEFINESBYTYPE('0000000000000000000006',$,$,$,(#4),#5);
#7=IFCWALL('0000000000000000000007',$,$,$,'Y',$,$,$,.USERDEFINED.);
#8=IFCWALLTYPE('0000000000000000000008',$,$,$,$,$,$,$,$,.NOTDEFINED.);
#9=IFCRELDEFINESBYTYPE('0000000000000000000009',$,$,$,(#7),#8);
#10=IFCWALL('0000000000000000000010',$,$,$,$,$,$,$,.PARTITIONING.);
#11=IFCWALLTYPE('0000000000000000000011',$,$,$,$,$,$,$,$,.SHEAR.);
#12=IFCRELDEFINESBYTYPE('0000000000000000000012',$,$,$,(#10),#11);
",
    );
    let resolved = |local| resolve(&session, local).unwrap();
    // The occurrence's own enumeration.
    assert_eq!(resolved("#1"), (Some("SOLIDWALL".into()), false));
    // User-defined: the occurrence's ObjectType.
    assert_eq!(resolved("#2"), (Some("WALDO".into()), true));
    // Nothing stated anywhere.
    assert_eq!(resolved("#3"), (None, false));
    // Inherited from a user-defined type: its ElementType.
    assert_eq!(resolved("#4"), (Some("X".into()), true));
    // A NOTDEFINED type defers to the occurrence.
    assert_eq!(resolved("#7"), (Some("Y".into()), true));
    // A defined type wins over the occurrence.
    assert_eq!(resolved("#10"), (Some("SHEAR".into()), false));
}

#[test]
fn an_occurrence_with_two_type_objects_is_refused() {
    let session = session(
        "#1=IFCWALL('0000000000000000000001',$,$,$,$,$,$,$,$);
#2=IFCWALLTYPE('0000000000000000000002',$,$,$,$,$,$,$,$,.SHEAR.);
#3=IFCWALLTYPE('0000000000000000000003',$,$,$,$,$,$,$,$,.SOLIDWALL.);
#4=IFCRELDEFINESBYTYPE('0000000000000000000004',$,$,$,(#1),#2);
#5=IFCRELDEFINESBYTYPE('0000000000000000000005',$,$,$,(#1),#3);
",
    );
    assert!(matches!(
        resolve(&session, "#1"),
        Err(PropertyResolutionError::Conflicting(_))
    ));
}

#[test]
fn other_names_in_the_reserved_set_are_absent() {
    let session = session("#1=IFCWALL('0000000000000000000001',$,$,$,$,$,$,$,.SOLIDWALL.);\n");
    assert_eq!(read(&session, "#1", "Nonsense"), Ok(None));
}
