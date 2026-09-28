//! Enumerated, list, bounded and table values map onto the IR's values;
//! reference values stay refused unless they reference nothing.
#![allow(missing_docs)]

use axioval_engine::{
    PropertyRequest, PropertyResolution, PropertyResolutionError, PropertyResolutionServiceHandle,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{
    Date, ObjectId, Property, PropertyTableRow, PropertyValue, QuantityDimension, SourceId,
};

/// Millimetre lengths. Wall #1 carries one property of each kind in
/// `Pset_Kinds`; door #30 a predefined panel set.
const IFC4: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#90=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#91=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);
#93=IFCUNITASSIGNMENT((#90));
#94=IFCPROJECT('0000000000000000000094',$,'P',$,$,$,$,$,#93);
#1=IFCWALL('0000000000000000000001',$,$,$,$,$,$,$,$);
#2=IFCPROPERTYENUMERATION('Status',(IFCLABEL('NEW'),IFCLABEL('EXISTING'),IFCLABEL('DEMOLISH')),$);
#3=IFCPROPERTYENUMERATEDVALUE('One',$,(IFCLABEL('EXISTING')),#2);
#4=IFCPROPERTYENUMERATEDVALUE('Two',$,(IFCLABEL('EXISTING'),IFCLABEL('DEMOLISH')),#2);
#5=IFCPROPERTYENUMERATEDVALUE('None',$,$,#2);
#6=IFCPROPERTYLISTVALUE('Labels',$,(IFCLABEL('X'),IFCLABEL('Y')),$);
#7=IFCPROPERTYLISTVALUE('Lengths',$,(IFCLENGTHMEASURE(1000.),IFCLENGTHMEASURE(2500.)),$);
#8=IFCPROPERTYLISTVALUE('Empty',$,$,$);
#9=IFCPROPERTYBOUNDEDVALUE('Range',$,IFCLENGTHMEASURE(5000.),IFCLENGTHMEASURE(1000.),$,IFCLENGTHMEASURE(3000.));
#10=IFCPROPERTYBOUNDEDVALUE('Above',$,$,IFCLENGTHMEASURE(2.),#91,$);
#11=IFCPROPERTYTABLEVALUE('Table',$,(IFCLABEL('X'),IFCLABEL('Y')),(IFCLENGTHMEASURE(1000.),IFCLENGTHMEASURE(2000.)),$,$,$,$);
#12=IFCPROPERTYREFERENCEVALUE('Reference',$,$,#94);
#13=IFCPROPERTYLISTVALUE('Dates',$,(IFCDATE('2026-09-27')),$);
#14=IFCPROPERTYTABLEVALUE('Ratios',$,(IFCREAL(1.),IFCREAL(2.)),(IFCREAL(0.5),IFCREAL(0.25)),$,$,$,$);
#15=IFCPROPERTYREFERENCEVALUE('Unreferenced',$,$,$);
#20=IFCPROPERTYSET('0000000000000000000020',$,'Pset_Kinds',$,(#3,#4,#5,#6,#7,#8,#9,#10,#11,#12,#13,#14,#15));
#21=IFCRELDEFINESBYPROPERTIES('0000000000000000000021',$,$,$,(#1),#20);
#30=IFCDOOR('0000000000000000000030',$,$,$,$,$,$,$,$,$,$,$,$);
#31=IFCDOORPANELPROPERTIES('0000000000000000000031',$,'Panel',$,$,.SWINGING.,$,.LEFT.,$);
#32=IFCRELDEFINESBYPROPERTIES('0000000000000000000032',$,$,$,(#30),#31);
ENDSEC;
END-ISO-10303-21;
";

fn resolve(
    object: &str,
    set: &str,
    name: &str,
) -> Result<PropertyResolution, PropertyResolutionError> {
    let session = import_ifc_session("model.ifc", IFC4.as_bytes()).unwrap();
    let request = PropertyRequest::try_new(
        ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), object).unwrap(),
        Some(set.to_owned()),
        name,
    )
    .unwrap();
    session
        .services()
        .get::<PropertyResolutionServiceHandle>()
        .unwrap()
        .resolve(&request)
}

fn property(name: &str) -> Property {
    match resolve("#1", "Pset_Kinds", name) {
        Ok(PropertyResolution::Present(resolved)) => resolved.property().clone(),
        other => panic!("Pset_Kinds.{name} is present: {other:?}"),
    }
}

fn text(value: &str) -> PropertyValue {
    PropertyValue::String(value.into())
}

fn metres(value: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value,
        dimension: QuantityDimension::Length,
    }
}

#[test]
fn an_enumerated_value_is_its_selected_item_or_a_list_of_them() {
    let one = property("One");
    assert_eq!(one.value, text("EXISTING"));
    assert_eq!(one.data_type(), Some("IFCLABEL"));
    let two = property("Two");
    assert_eq!(
        two.value,
        PropertyValue::List(vec![text("EXISTING"), text("DEMOLISH")])
    );
    assert_eq!(two.data_type(), Some("IFCLABEL"));
    // Nothing selected is no value, not an absent property.
    let none = property("None");
    assert_eq!(none.value, PropertyValue::Null);
    assert_eq!(none.data_type(), Some("IFCLABEL"));
}

#[test]
fn a_list_value_is_a_list_in_si() {
    assert_eq!(
        property("Labels").value,
        PropertyValue::List(vec![text("X"), text("Y")])
    );
    let lengths = property("Lengths");
    assert_eq!(
        lengths.value,
        PropertyValue::List(vec![metres(1.0), metres(2.5)])
    );
    assert_eq!(lengths.data_type(), Some("IFCLENGTHMEASURE"));
    assert_eq!(property("Empty").value, PropertyValue::Null);
    assert_eq!(
        property("Dates").value,
        PropertyValue::List(vec![PropertyValue::Date(
            "2026-09-27".parse::<Date>().unwrap()
        )])
    );
}

#[test]
fn a_bounded_value_keeps_its_bounds_and_set_point() {
    let range = property("Range");
    assert_eq!(
        range.value,
        PropertyValue::Bounded {
            lower: Some(Box::new(metres(1.0))),
            upper: Some(Box::new(metres(5.0))),
            set_point: Some(Box::new(metres(3.0))),
        }
    );
    assert_eq!(range.data_type(), Some("IFCLENGTHMEASURE"));
    // The explicit metre unit applies; the upper end stays open.
    assert_eq!(
        property("Above").value,
        PropertyValue::Bounded {
            lower: Some(Box::new(metres(2.0))),
            upper: None,
            set_point: None,
        }
    );
}

#[test]
fn a_table_value_keeps_its_rows_and_a_type_only_when_both_columns_share_it() {
    let table = property("Table");
    assert_eq!(
        table.value,
        PropertyValue::Table(vec![
            PropertyTableRow {
                defining: text("X"),
                defined: metres(1.0),
            },
            PropertyTableRow {
                defining: text("Y"),
                defined: metres(2.0),
            },
        ])
    );
    assert_eq!(table.data_type(), None);
    // Each column's own type is reported instead.
    let columns = table.column_types().unwrap();
    assert_eq!(
        (columns.defining.as_str(), columns.defined.as_str()),
        ("IFCLABEL", "IFCLENGTHMEASURE")
    );
    let ratios = property("Ratios");
    assert_eq!(ratios.data_type(), Some("IFCREAL"));
    assert_eq!(
        ratios.value.stated_values().unwrap(),
        [
            &PropertyValue::Decimal(1.0),
            &PropertyValue::Decimal(0.5),
            &PropertyValue::Decimal(2.0),
            &PropertyValue::Decimal(0.25),
        ]
    );
}

#[test]
fn a_reference_value_stays_refused() {
    let result = resolve("#1", "Pset_Kinds", "Reference");
    assert!(result.is_err(), "{result:?}");
}

#[test]
fn a_reference_value_referencing_nothing_is_no_value() {
    let unreferenced = property("Unreferenced");
    assert_eq!(unreferenced.value, PropertyValue::Null);
    assert_eq!(unreferenced.data_type(), None);
}

#[test]
fn an_enumeration_attribute_of_a_predefined_set_is_its_constant() {
    let Ok(PropertyResolution::Present(resolved)) = resolve("#30", "Panel", "PanelOperation")
    else {
        panic!("Panel.PanelOperation is present");
    };
    assert_eq!(resolved.property().value, text("SWINGING"));
    assert_eq!(
        resolved.property().data_type(),
        Some("IFCDOORPANELOPERATIONENUM")
    );
}
