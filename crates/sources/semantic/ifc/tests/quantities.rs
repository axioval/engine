//! Quantity sets resolve like property sets, in SI; what is not read is refused.
#![allow(missing_docs)]

use axioval_engine::{
    NameMatch, PropertyEnumerationRequest, PropertyRequest, PropertyResolution,
    PropertyResolutionError, PropertyResolutionServiceHandle,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{ObjectId, PropertyValue, QuantityDimension, SourceId};

/// Millimetre lengths and square-metre areas. Space #1 carries base
/// quantities, one with an explicit metre unit, a count, and a complex
/// quantity; door #10 carries a predefined lining set with one attribute
/// stated; wall #20 has a
/// property set and a quantity set of one name. Slabs #30, #33 and #36 each
/// carry one quantity without a readable value: `$`, text, and a record cut
/// off before its value. Wall #40 states an `IfcNumericMeasure`.
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
#30=IFCSLAB('0000000000000000000030',$,$,$,$,$,$,$,$);
#31=IFCQUANTITYLENGTH('Width',$,$,$,$);
#32=IFCELEMENTQUANTITY('0000000000000000000032',$,'Qto_SlabBaseQuantities',$,$,(#31));
#39=IFCRELDEFINESBYPROPERTIES('0000000000000000000039',$,$,$,(#30),#32);
#33=IFCSLAB('0000000000000000000033',$,$,$,$,$,$,$,$);
#34=IFCQUANTITYLENGTH('Width',$,$,'wide',$);
#35=IFCELEMENTQUANTITY('0000000000000000000035',$,'Qto_SlabBaseQuantities',$,$,(#34));
#49=IFCRELDEFINESBYPROPERTIES('0000000000000000000049',$,$,$,(#33),#35);
#36=IFCSLAB('0000000000000000000036',$,$,$,$,$,$,$,$);
#37=IFCQUANTITYLENGTH('Width',$,$);
#38=IFCELEMENTQUANTITY('0000000000000000000038',$,'Qto_SlabBaseQuantities',$,$,(#37));
#59=IFCRELDEFINESBYPROPERTIES('0000000000000000000059',$,$,$,(#36),#38);
#40=IFCWALL('0000000000000000000040',$,$,$,$,$,$,$,$);
#41=IFCPROPERTYSINGLEVALUE('Ratio',$,IFCNUMERICMEASURE(3.5),$);
#42=IFCPROPERTYSET('0000000000000000000042',$,'Pset_Numbers',$,(#41));
#43=IFCRELDEFINESBYPROPERTIES('0000000000000000000043',$,$,$,(#40),#42);
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
fn a_predefined_set_attribute_is_a_measure_in_si() {
    // 100 mm, stated as an `IfcDoorLiningProperties` attribute.
    for set in [Some("Lining"), None] {
        let Ok(PropertyResolution::Present(resolved)) = resolve("#10", set, "LiningDepth") else {
            panic!("{set:?}.LiningDepth is present");
        };
        let property = resolved.property();
        let PropertyValue::Quantity { value, dimension } = property.value else {
            panic!("{set:?}.LiningDepth is a quantity: {property:?}");
        };
        assert!((value - 0.1).abs() < 1e-12, "{value}");
        assert_eq!(dimension, QuantityDimension::Length);
        assert_eq!(property.property_set, "Lining");
    }
}

#[test]
fn what_is_not_read_is_refused_never_absent() {
    for (object, set, name) in [
        // A complex quantity of the requested name.
        ("#1", QTO, "Layer"),
        // A property set and a quantity set of one name.
        ("#20", Some("Common"), "Width"),
    ] {
        let result = resolve(object, set, name);
        assert!(result.is_err(), "{object} {set:?}.{name}: {result:?}");
    }
}

#[test]
fn an_unset_attribute_of_a_predefined_set_is_null_with_its_declared_type() {
    let Ok(PropertyResolution::Present(resolved)) =
        resolve("#10", Some("Lining"), "LiningThickness")
    else {
        panic!("Lining.LiningThickness is present, unset");
    };
    assert_eq!(resolved.property().value, PropertyValue::Null);
    assert_eq!(
        resolved.property().data_type(),
        Some("IFCNONNEGATIVELENGTHMEASURE")
    );
}

/// A quantity whose value is `$`, not a number, or cut off by a truncated
/// record is never read as 0 and never as absent: resolving it and
/// enumerating its set both refuse, so a rule over it is not evaluated.
#[test]
fn a_quantity_without_a_readable_value_is_refused_never_zero_or_absent() {
    let session = import_ifc_session("model.ifc", IFC4.as_bytes()).unwrap();
    let service = session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap();
    for (slab, why) in [("#30", "`$`"), ("#33", "text"), ("#36", "truncated")] {
        for set in [Some("Qto_SlabBaseQuantities"), None] {
            let result = resolve(slab, set, "Width");
            assert!(result.is_err(), "{why} {set:?}.Width: {result:?}");
        }
        let request = PropertyEnumerationRequest::try_new(
            ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), slab).unwrap(),
            NameMatch::Exact("Qto_SlabBaseQuantities".into()),
            NameMatch::Exact("Width".into()),
        )
        .unwrap();
        let enumerated = service.enumerate(&request);
        assert!(enumerated.is_err(), "{why} enumerated: {enumerated:?}");
    }
}

/// `IfcNumericMeasure`, the measure of IFC4X3 `IfcQuantityNumber`, is a plain
/// number without a unit.
#[test]
fn a_numeric_measure_is_a_plain_number() {
    assert_eq!(
        value("#40", Some("Pset_Numbers"), "Ratio"),
        PropertyValue::Decimal(3.5)
    );
}
