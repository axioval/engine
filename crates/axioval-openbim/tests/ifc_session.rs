//! Production IFC evidence-session integration tests.

use axioval_engine::{
    PropertyRequest, PropertyResolution, PropertyResolutionError, PropertyResolutionServiceHandle,
    RelationshipSelectionServiceHandle,
};
use axioval_ir::{ObjectId, PropertyValue, SourceId};
use axioval_openbim::import_ifc_session;

const IFC: &[u8] = b"ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n#1=IFCWALL('g',$,$,$,$,$,$,$,$);\n#2=IFCPROPERTYSINGLEVALUE('Flag',$,ifcboolean(.T.),$);\n#3=IFCPROPERTYSINGLEVALUE('Big',$,IFCINTEGER(9007199254740993),$);\n#4=IFCPROPERTYSET('p',$,'Pset_Test',$,(#2,#3,#6,#7,#8,#10));\n#5=IFCRELDEFINESBYPROPERTIES('r',$,$,$,(#1),#4);\n#6=IFCPROPERTYSINGLEVALUE('Logical',$,IFCLOGICAL(.U.),$);\n#7=IFCPROPERTYSINGLEVALUE('Bits',$,IFCBINARY(\"0101\"),$);\n#8=IFCPROPERTYSINGLEVALUE('Length',$,IFCLENGTHMEASURE(1.),$);\n#9=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n#10=IFCPROPERTYSINGLEVALUE('UnitReal',$,IFCREAL(1.),#9);\nENDSEC;\nEND-ISO-10303-21;\n";

fn request(name: &str) -> PropertyRequest {
    PropertyRequest::try_new(
        ObjectId::new(SourceId::new("ifc-step", "fixture.ifc").unwrap(), "#1").unwrap(),
        Some("Pset_Test".to_string()),
        name,
    )
    .unwrap()
}

#[test]
fn strict_ifc_bytes_build_an_exact_direct_property_session() {
    let session = import_ifc_session("fixture.ifc", IFC).unwrap();
    assert_eq!(session.project().objects().count(), 1);
    let snapshot = session.snapshots().next().unwrap();
    assert_eq!(snapshot.schema(), Some("IFC4"));
    assert!(snapshot.fingerprint().starts_with("sha256:"));
    assert_eq!(snapshot.revision(), snapshot.fingerprint());
    let fingerprint = snapshot.fingerprint().to_owned();
    assert!(
        session
            .service::<RelationshipSelectionServiceHandle>()
            .is_none()
    );

    let properties = session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap();
    let PropertyResolution::Present(flag) = properties.resolve(&request("Flag")).unwrap() else {
        panic!("flag must be present");
    };
    assert_eq!(flag.property().value, PropertyValue::Boolean(true));
    let flag_evidence = flag.property().evidence.as_ref().unwrap();
    assert_eq!(flag_evidence.source, request("Flag").object_id().source);
    assert!(flag_evidence.locator.contains(&fingerprint));
    assert!(flag_evidence.locator.contains("#4/#2"));

    let PropertyResolution::Present(big) = properties.resolve(&request("Big")).unwrap() else {
        panic!("integer must be present");
    };
    assert_eq!(
        big.property().value,
        PropertyValue::Integer(9_007_199_254_740_993)
    );

    let PropertyResolution::Absent(absence) = properties.resolve(&request("Missing")).unwrap()
    else {
        panic!("missing property must have exact absence proof");
    };
    assert!(absence.evidence().exact);
    assert!(!absence.evidence().locator.trim().is_empty());
    assert_eq!(
        absence.evidence().source,
        request("Missing").object_id().source
    );

    let foreign = PropertyRequest::try_new(
        ObjectId::new(SourceId::new("ifc", "other.ifc").unwrap(), "#1").unwrap(),
        Some("Pset_Test".to_string()),
        "Flag",
    )
    .unwrap();
    assert_eq!(
        properties.resolve(&foreign),
        Err(PropertyResolutionError::InvalidRequest)
    );
}

#[test]
fn unsupported_source_neutral_scalars_fail_closed() {
    let session = import_ifc_session("fixture.ifc", IFC).unwrap();
    let service = session
        .services()
        .get::<PropertyResolutionServiceHandle>()
        .unwrap();

    for name in ["Logical", "Bits", "Length", "UnitReal"] {
        assert_eq!(
            service.resolve(&request(name)),
            Err(PropertyResolutionError::InexactEvidence)
        );
    }
}

#[test]
fn source_failures_remain_distinct_terminal_errors() {
    let incomplete = String::from_utf8(IFC.to_vec())
        .unwrap()
        .replace("(#2,#3,#6,#7,#8,#10)", "()");
    let session = import_ifc_session("fixture.ifc", incomplete.as_bytes()).unwrap();
    let properties = session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap();
    assert!(matches!(
        properties.resolve(&request("Missing")),
        Err(PropertyResolutionError::Incomplete(_))
    ));

    let conflicting = String::from_utf8(IFC.to_vec())
        .unwrap()
        .replace("'Big'", "'Flag'");
    let session = import_ifc_session("fixture.ifc", conflicting.as_bytes()).unwrap();
    let properties = session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap();
    assert!(matches!(
        properties.resolve(&request("Flag")),
        Err(PropertyResolutionError::Conflicting(_))
    ));

    let inexact = String::from_utf8(IFC.to_vec())
        .unwrap()
        .replace("ifcboolean(.T.)", "(1,2)");
    let session = import_ifc_session("fixture.ifc", inexact.as_bytes()).unwrap();
    let properties = session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap();
    assert_eq!(
        properties.resolve(&request("Flag")),
        Err(PropertyResolutionError::InexactEvidence)
    );
}

#[test]
fn malformed_or_non_ifc4_input_cannot_create_an_exact_session() {
    assert!(import_ifc_session("broken.ifc", b"not STEP").is_err());
    let ifc2x3 = String::from_utf8(IFC.to_vec())
        .unwrap()
        .replace("FILE_SCHEMA(('IFC4'))", "FILE_SCHEMA(('IFC2X3'))");
    assert!(import_ifc_session("old.ifc", ifc2x3.as_bytes()).is_err());
}
