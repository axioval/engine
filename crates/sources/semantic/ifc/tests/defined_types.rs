//! Values of defined types are carried when their base maps without loss.
#![allow(missing_docs)]

use axioval_engine::{
    PropertyRequest, PropertyResolution, PropertyResolutionError, PropertyResolutionServiceHandle,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{ObjectId, PropertyValue, SourceId};

const IFC4: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCWALL('0000000000000000000001',$,$,$,$,$,$,$,$);
#2=IFCPROPERTYSINGLEVALUE('Date',$,IFCDATE('2022-01-01'),$);
#3=IFCPROPERTYSINGLEVALUE('Count',$,IFCCOUNTMEASURE(3),$);
#4=IFCPROPERTYSINGLEVALUE('Stamp',$,IFCTIMESTAMP(1700000000),$);
#5=IFCPROPERTYSINGLEVALUE('Length',$,IFCLENGTHMEASURE(2.5),$);
#6=IFCPROPERTYSINGLEVALUE('Duration',$,IFCDURATION('P1D'),$);
#7=IFCPROPERTYSET('0000000000000000000002',$,'P',$,(#2,#3,#4,#5,#6));
#8=IFCRELDEFINESBYPROPERTIES('0000000000000000000003',$,$,$,(#1),#7);
ENDSEC;
END-ISO-10303-21;
";

fn resolve(name: &str) -> Result<PropertyResolution, PropertyResolutionError> {
    let session = import_ifc_session("model.ifc", IFC4.as_bytes()).unwrap();
    let request = PropertyRequest::try_new(
        ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), "#1").unwrap(),
        Some("P".to_owned()),
        name,
    )
    .unwrap();
    session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap()
        .resolve(&request)
}

#[test]
fn string_and_integer_based_types_are_carried_with_their_declared_type() {
    for (name, value, data_type) in [
        (
            "Date",
            PropertyValue::String("2022-01-01".into()),
            "IFCDATE",
        ),
        (
            "Duration",
            PropertyValue::String("P1D".into()),
            "IFCDURATION",
        ),
        ("Count", PropertyValue::Integer(3), "IFCCOUNTMEASURE"),
        (
            "Stamp",
            PropertyValue::Integer(1_700_000_000),
            "IFCTIMESTAMP",
        ),
    ] {
        let Ok(PropertyResolution::Present(resolved)) = resolve(name) else {
            panic!("{name}: {:?}", resolve(name));
        };
        assert_eq!(resolved.property().value, value, "{name}");
        assert_eq!(resolved.property().data_type(), Some(data_type), "{name}");
    }
}

#[test]
fn a_measure_with_a_unit_context_stays_refused() {
    assert!(matches!(
        resolve("Length"),
        Err(PropertyResolutionError::InexactEvidence)
    ));
}
