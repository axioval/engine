//! A storey's height to the next storey of its building, answered as the
//! measured value `level_height`.
#![allow(missing_docs)]

use axioval_engine::{
    PropertyRequest, PropertyResolution, PropertyResolutionError, PropertyResolutionServiceHandle,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{MEASURED_SET, ObjectId, PropertyValue, QuantityDimension, SourceId};

/// Millimetres. Storeys at 0, 3000 and 6500 mm in one building (listed out
/// of order), a storey of another building at 3000 mm, and a wall.
const DATA: &str = "\
#1=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#2=IFCUNITASSIGNMENT((#1));
#3=IFCPROJECT('0000000000000000000003',$,'P',$,$,$,$,$,#2);
#10=IFCCARTESIANPOINT((0.,0.,0.));
#11=IFCAXIS2PLACEMENT3D(#10,$,$);
#12=IFCLOCALPLACEMENT($,#11);
#13=IFCBUILDING('000000000000000000000D',$,'A',$,$,#12,$,$,.ELEMENT.,$,$,$);
#14=IFCBUILDING('000000000000000000000E',$,'B',$,$,#12,$,$,.ELEMENT.,$,$,$);
#20=IFCCARTESIANPOINT((0.,0.,3000.));
#21=IFCAXIS2PLACEMENT3D(#20,$,$);
#22=IFCLOCALPLACEMENT(#12,#21);
#23=IFCCARTESIANPOINT((0.,0.,6500.));
#24=IFCAXIS2PLACEMENT3D(#23,$,$);
#25=IFCLOCALPLACEMENT(#12,#24);
#30=IFCBUILDINGSTOREY('000000000000000000000G',$,'0',$,$,#12,$,$,.ELEMENT.,0.);
#31=IFCBUILDINGSTOREY('000000000000000000000H',$,'2',$,$,#25,$,$,.ELEMENT.,6500.);
#32=IFCBUILDINGSTOREY('000000000000000000000I',$,'1',$,$,#22,$,$,.ELEMENT.,3000.);
#33=IFCBUILDINGSTOREY('000000000000000000000J',$,'B1',$,$,#22,$,$,.ELEMENT.,3000.);
#40=IFCRELAGGREGATES('000000000000000000000K',$,$,$,#13,(#30,#31,#32));
#41=IFCRELAGGREGATES('000000000000000000000L',$,$,$,#14,(#33));
#50=IFCWALL('000000000000000000000X',$,'W',$,$,#12,$,$,$);
";

fn level_height(local: &str) -> Result<PropertyResolution, PropertyResolutionError> {
    let file = format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n{DATA}ENDSEC;\nEND-ISO-10303-21;\n"
    );
    let session = import_ifc_session("model.ifc", file.as_bytes()).unwrap();
    let object = ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), local).unwrap();
    let request =
        PropertyRequest::try_new(object, Some(MEASURED_SET.into()), "level_height").unwrap();
    session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap()
        .resolve(&request)
}

fn metres(resolution: &PropertyResolution) -> f64 {
    let PropertyResolution::Present(resolved) = resolution else {
        panic!("absent: {resolution:?}");
    };
    let PropertyValue::Quantity { value, dimension } = resolved.property().value else {
        panic!("not a quantity: {resolved:?}");
    };
    assert_eq!(dimension, QuantityDimension::Length);
    assert!(resolved.property().evidence.as_ref().unwrap().exact);
    value
}

#[test]
fn a_storey_is_as_high_as_the_next_storey_of_its_building_is_above_it() {
    assert!((metres(&level_height("#30").unwrap()) - 3.0).abs() < 1e-12);
    assert!((metres(&level_height("#32").unwrap()) - 3.5).abs() < 1e-12);
}

#[test]
fn the_highest_storey_and_other_objects_have_no_level_height() {
    // #33 is alone in its building; #31 is the top of its own.
    for local in ["#31", "#33", "#50"] {
        assert!(
            matches!(level_height(local), Ok(PropertyResolution::Absent(_))),
            "{local}"
        );
    }
}

#[test]
fn a_sibling_at_the_same_elevation_is_refused() {
    let file = DATA.replace(
        "#41=IFCRELAGGREGATES('000000000000000000000L',$,$,$,#14,(#33));",
        "#41=IFCRELAGGREGATES('000000000000000000000L',$,$,$,#13,(#33));",
    );
    let file = format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n{file}ENDSEC;\nEND-ISO-10303-21;\n"
    );
    let session = import_ifc_session("model.ifc", file.as_bytes()).unwrap();
    let object = ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), "#32").unwrap();
    let request =
        PropertyRequest::try_new(object, Some(MEASURED_SET.into()), "level_height").unwrap();
    assert!(matches!(
        session
            .service::<PropertyResolutionServiceHandle>()
            .unwrap()
            .resolve(&request),
        Err(PropertyResolutionError::Unavailable(_))
    ));
}
