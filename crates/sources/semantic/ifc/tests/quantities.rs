//! Quantity sets resolve like property sets, in SI; what is not read is refused.
#![allow(missing_docs)]

use axioval_engine::{
    PropertyRequest, PropertyResolution, PropertyResolutionError, PropertyResolutionServiceHandle,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{ObjectId, PropertyValue, QuantityDimension, SourceId};

/// Millimetre lengths and square-metre areas. Space #1 carries base
/// quantities, one with an explicit metre unit, a count, and a complex
/// quantity; door #10 carries a predefined lining set; wall #20 has a
/// property set and a quantity set of one name.
const IFC4: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#90=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#91=IFCSIUNIT(*,.AREAUNIT.,$,.SQUARE_METRE.);
#92=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);
#93=IFCUNITASSIGNMENT((#90,#91));
#94=IFCPROJECT('0000000000000000000094',$,'P',$,$,$,$,$,#93);
#1=IFCSPACE('0000000000000000000001',$,'101',$,$,$,$,$,.ELEMENT.,$,$);
#2=IFCQUANTITYAREA('NetFloorArea',$,$,12.5,$);
#3=IFCQUANTITYLENGTH('Height',$,$,2750.,$);
#4=IFCQUANTITYLENGTH('Perimeter',$,#92,14.,$);
#5=IFCQUANTITYCOUNT('Doors',$,$,2.,$);
#6=IFCPHYSICALCOMPLEXQUANTITY('Layer',$,(#7),'layer',$,$);
#7=IFCQUANTITYLENGTH('Inner',$,$,1.,$);
#8=IFCELEMENTQUANTITY('0000000000000000000008',$,'Qto_SpaceBaseQuantities',$,$,(#2,#3,#4,#5,#6));
#9=IFCRELDEFINESBYPROPERTIES('0000000000000000000009',$,$,$,(#1),#8);
#10=IFCDOOR('0000000000000000000010',$,$,$,$,$,$,$,$,$,$,$,$);
#11=IFCDOORLININGPROPERTIES('0000000000000000000011',$,'Lining',$,100.,$,$,$,$,$,$,$,$,$,$,$,$);
#12=IFCRELDEFINESBYPROPERTIES('0000000000000000000012',$,$,$,(#10),#11);
#20=IFCWALL('0000000000000000000020',$,$,$,$,$,$,$,$);
#21=IFCPROPERTYSINGLEVALUE('Width',$,IFCLABEL('thick'),$);
#22=IFCPROPERTYSET('0000000000000000000022',$,'Common',$,(#21));
#23=IFCQUANTITYLENGTH('Width',$,$,240.,$);
#24=IFCELEMENTQUANTITY('0000000000000000000024',$,'Common',$,$,(#23));
#25=IFCRELDEFINESBYPROPERTIES('0000000000000000000025',$,$,$,(#20),#22);
#26=IFCRELDEFINESBYPROPERTIES('0000000000000000000026',$,$,$,(#20),#24);
ENDSEC;
END-ISO-10303-21;
";

fn resolve(
    object: &str,
    set: Option<&str>,
    name: &str,
) -> Result<PropertyResolution, PropertyResolutionError> {
    let session = import_ifc_session("model.ifc", IFC4.as_bytes()).unwrap();
    let request = PropertyRequest::try_new(
        ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), object).unwrap(),
        set.map(ToOwned::to_owned),
        name,
    )
    .unwrap();
    session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap()
        .resolve(&request)
}

fn value(object: &str, set: Option<&str>, name: &str) -> PropertyValue {
    match resolve(object, set, name) {
        Ok(PropertyResolution::Present(resolved)) => resolved.property().value.clone(),
        other => panic!("{set:?}.{name}: {other:?}"),
    }
}

const QTO: Option<&str> = Some("Qto_SpaceBaseQuantities");

#[test]
fn base_quantities_are_quantities_in_si() {
    assert_eq!(
        value("#1", QTO, "NetFloorArea"),
        PropertyValue::Quantity {
            value: 12.5,
            dimension: QuantityDimension::Area
        }
    );
    // 2750 mm in the project's length unit.
    let PropertyValue::Quantity { value: height, .. } = value("#1", QTO, "Height") else {
        panic!("height is a quantity");
    };
    assert!((height - 2.75).abs() < 1e-12, "{height}");
    // An explicit metre unit wins over the millimetre default.
    let PropertyValue::Quantity {
        value: perimeter, ..
    } = value("#1", QTO, "Perimeter")
    else {
        panic!("perimeter is a quantity");
    };
    assert!((perimeter - 14.0).abs() < 1e-12, "{perimeter}");
    // Quantities are found without naming their set, too.
    assert!(matches!(
        value("#1", None, "NetFloorArea"),
        PropertyValue::Quantity { .. }
    ));
}

#[test]
fn a_count_is_a_plain_number() {
    assert_eq!(value("#1", QTO, "Doors"), PropertyValue::Decimal(2.0));
}

#[test]
fn a_read_quantity_set_proves_absence() {
    assert!(matches!(
        resolve("#1", QTO, "GrossVolume"),
        Ok(PropertyResolution::Absent(_))
    ));
}

#[test]
fn what_is_not_read_is_refused_never_absent() {
    for (object, set, name) in [
        // A complex quantity of the requested name.
        ("#1", QTO, "Layer"),
        // An attribute of a predefined property set.
        ("#10", Some("Lining"), "LiningDepth"),
        ("#10", None, "LiningDepth"),
        // A property set and a quantity set of one name.
        ("#20", Some("Common"), "Width"),
    ] {
        let result = resolve(object, set, name);
        assert!(result.is_err(), "{object} {set:?}.{name}: {result:?}");
    }
}
