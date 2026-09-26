//! Direct attributes are read from the object's own instance, per release.
#![allow(missing_docs)]

use axioval_engine::{AttributeError, AttributeServiceHandle, AttributeValue};
use axioval_ifc::import_ifc_session;
use axioval_ir::{ObjectId, PropertyValue, SourceId};

fn session(schema: &str, data: &str) -> axioval_engine::EvidenceSession {
    let bytes = format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('{schema}'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
    );
    import_ifc_session("model.ifc", bytes.as_bytes()).unwrap()
}

fn read(
    session: &axioval_engine::EvidenceSession,
    local: &str,
    name: &str,
) -> Result<AttributeValue, AttributeError> {
    let id = ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), local).unwrap();
    session
        .service::<AttributeServiceHandle>()
        .unwrap()
        .attribute(&id, name)
        .map(|resolved| {
            assert!(resolved.evidence.exact);
            assert!(
                resolved
                    .evidence
                    .locator
                    .contains(&format!("attribute:{local}.{name}"))
            );
            resolved.value
        })
}

fn scalar(value: PropertyValue, data_type: &str) -> AttributeValue {
    AttributeValue::Scalar {
        value,
        data_type: Some(data_type.to_owned()),
    }
}

const IFC4: &str = "#1=IFCWALL('0000000000000000000001',$,'W-1','',$,#2,$,$,.SHEAR.);
#2=IFCLOCALPLACEMENT($,#3);
#3=IFCAXIS2PLACEMENT3D(#4,$,$);
#4=IFCCARTESIANPOINT((0.,0.,0.));
#5=IFCDOOR('0000000000000000000002',$,$,$,$,$,$,$,2.1,$,$,$,$);
";

#[test]
fn scalars_carry_their_declared_type() {
    let session = session("IFC4", IFC4);
    assert_eq!(
        read(&session, "#1", "Name"),
        Ok(scalar(PropertyValue::String("W-1".into()), "IFCLABEL"))
    );
    // An enumeration item is its text.
    assert_eq!(
        read(&session, "#1", "PredefinedType"),
        Ok(scalar(
            PropertyValue::String("SHEAR".into()),
            "IFCWALLTYPEENUM"
        ))
    );
    // An empty string is a value; whether it satisfies anything is policy.
    assert_eq!(
        read(&session, "#1", "Description"),
        Ok(scalar(PropertyValue::String(String::new()), "IFCTEXT"))
    );
}

#[test]
fn unset_structured_and_unreadable_values_are_distinguished() {
    let session = session("IFC4", IFC4);
    assert_eq!(
        read(&session, "#1", "ObjectType"),
        Ok(AttributeValue::Unset)
    );
    assert_eq!(
        read(&session, "#1", "ObjectPlacement"),
        Ok(AttributeValue::Structured)
    );
    // A length means nothing until the unit context is read.
    assert!(matches!(
        read(&session, "#5", "OverallHeight"),
        Err(AttributeError::Unsupported(_))
    ));
    assert!(matches!(
        read(&session, "#1", "Nonsense"),
        Err(AttributeError::UnknownAttribute { .. })
    ));
    // Attribute names are case-sensitive, as the schema spells them.
    assert!(matches!(
        read(&session, "#1", "name"),
        Err(AttributeError::UnknownAttribute { .. })
    ));
}

#[test]
fn ifc2x3_slots_come_from_the_ifc2x3_schema() {
    // IFC2X3 IfcWall has no PredefinedType; its eighth attribute is Tag.
    let session = session(
        "IFC2X3",
        "#1=IFCWALL('0000000000000000000001',$,'W-1',$,$,$,$,'T-9');\n",
    );
    assert_eq!(
        read(&session, "#1", "Tag"),
        Ok(scalar(PropertyValue::String("T-9".into()), "IFCIDENTIFIER"))
    );
    assert!(matches!(
        read(&session, "#1", "PredefinedType"),
        Err(AttributeError::UnknownAttribute { .. })
    ));
}
