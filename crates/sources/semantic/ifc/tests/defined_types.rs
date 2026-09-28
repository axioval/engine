//! Values of defined types are carried when their base maps without loss.
#![allow(missing_docs)]

use axioval_engine::{
    PropertyRequest, PropertyResolution, PropertyResolutionError, PropertyResolutionServiceHandle,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{ATTRIBUTE_SET, ObjectId, PropertyValue, SourceId};

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
#7=IFCPROPERTYSET('0000000000000000000002',$,'P',$,(#2,#3,#4,#5,#6,#9,#10,#11,#12,#13,#15,#16,#18,#19,#20,#21));
#8=IFCRELDEFINESBYPROPERTIES('0000000000000000000003',$,$,$,(#1),#7);
#9=IFCPROPERTYSINGLEVALUE('Inspected',$,IFCDATETIME('2026-09-27T10:30:00.25+02:00'),$);
#10=IFCPROPERTYSINGLEVALUE('Local',$,IFCDATETIME('2026-09-27T10:30:00'),$);
#11=IFCPROPERTYSINGLEVALUE('BadDate',$,IFCDATE('27.09.2026'),$);
#12=IFCPROPERTYSINGLEVALUE('NoDay',$,IFCDATE('2026-02-30'),$);
#13=IFCPROPERTYSINGLEVALUE('BadDateTime',$,IFCDATETIME('2026-09-27 10:30'),$);
#14=IFCWORKPLAN('0000000000000000000004',$,'Plan',$,$,$,'2026-09-27T08:00:00Z',$,$,$,$,'2026-10-01T07:00:00',$,$);
#15=IFCPROPERTYSINGLEVALUE('ZonedDate',$,IFCDATE('2022-01-01+00:00'),$);
#16=IFCPROPERTYSINGLEVALUE('FarZone',$,IFCDATE('2022-01-01+14:30'),$);
#17=IFCCLASSIFICATION($,$,'2022-01-01-05:00','Name',$,$,$);
#18=IFCPROPERTYSINGLEVALUE('True',$,IFCLOGICAL(.T.),$);
#19=IFCPROPERTYSINGLEVALUE('False',$,IFCLOGICAL(.F.),$);
#20=IFCPROPERTYSINGLEVALUE('Unknown',$,IFCLOGICAL(.U.),$);
#21=IFCPROPERTYLISTVALUE('Unknowns',$,(IFCLOGICAL(.T.),IFCLOGICAL(.U.)),$);
ENDSEC;
END-ISO-10303-21;
";

fn resolve(name: &str) -> Result<PropertyResolution, PropertyResolutionError> {
    resolve_in("#1", "P", name)
}

fn resolve_in(
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
        .service::<PropertyResolutionServiceHandle>()
        .unwrap()
        .resolve(&request)
}

#[test]
fn string_and_integer_based_types_are_carried_with_their_declared_type() {
    for (name, value, data_type) in [
        (
            "Duration",
            PropertyValue::String("P1D".into()),
            "IFCDURATION",
        ),
        ("Count", PropertyValue::Integer(3), "IFCCOUNTMEASURE"),
    ] {
        let Ok(PropertyResolution::Present(resolved)) = resolve(name) else {
            panic!("{name}: {:?}", resolve(name));
        };
        assert_eq!(resolved.property().value, value, "{name}");
        assert_eq!(resolved.property().data_type(), Some(data_type), "{name}");
    }
}

/// The declared type and locator of a present property whose value cannot
/// be read, and why.
fn unreadable(name: &str) -> (String, String, String) {
    let Err(PropertyResolutionError::UnreadableValue(unreadable)) = resolve(name) else {
        panic!("{name}: {:?}", resolve(name));
    };
    assert_eq!(unreadable.request().property(), name);
    assert!(unreadable.evidence().exact);
    (
        unreadable.data_type().to_owned(),
        unreadable.evidence().locator.clone(),
        unreadable.reason().to_owned(),
    )
}

#[test]
fn a_measure_without_a_unit_to_convert_from_keeps_its_declared_type() {
    // Measures convert to SI through their unit (tests/measures.rs); this
    // file has no project, so there is no default length unit. The value is
    // stated all the same, so the property is present and typed.
    let (data_type, locator, reason) = unreadable("Length");
    assert_eq!(data_type, "IFCLENGTHMEASURE");
    assert!(locator.ends_with(":occurrence:#7/#5"), "{locator}");
    assert!(
        reason.contains("the unit of a IFCLENGTHMEASURE cannot be resolved exactly"),
        "{reason}"
    );
}

fn present(object: &str, set: &str, name: &str) -> (PropertyValue, Option<String>, String) {
    let Ok(PropertyResolution::Present(resolved)) = resolve_in(object, set, name) else {
        panic!("{name}: {:?}", resolve_in(object, set, name));
    };
    let property = resolved.property();
    (
        property.value.clone(),
        property.data_type().map(ToOwned::to_owned),
        property.evidence.as_ref().unwrap().locator.clone(),
    )
}

#[test]
fn dates_date_times_and_time_stamps_are_dates_with_their_declared_type() {
    let date_time = |text: &str| PropertyValue::DateTime(text.parse().unwrap());
    for (name, expected, data_type, locator) in [
        (
            "Date",
            PropertyValue::Date("2022-01-01".parse().unwrap()),
            "IFCDATE",
            ":occurrence:#7/#2",
        ),
        (
            "Inspected",
            date_time("2026-09-27T10:30:00.25+02:00"),
            "IFCDATETIME",
            ":occurrence:#7/#9",
        ),
        // `IfcDate` is an `xs:date`, which may state a time zone.
        (
            "ZonedDate",
            PropertyValue::Date("2022-01-01Z".parse().unwrap()),
            "IFCDATE",
            ":occurrence:#7/#15",
        ),
        // Seconds since the epoch, in UTC.
        (
            "Stamp",
            date_time("2023-11-14T22:13:20Z"),
            "IFCTIMESTAMP",
            ":occurrence:#7/#4",
        ),
    ] {
        let (value, declared, found) = present("#1", "P", name);
        assert_eq!(value, expected, "{name}");
        assert_eq!(declared.as_deref(), Some(data_type), "{name}");
        assert!(found.ends_with(locator), "{name}: {found}");
    }
}

#[test]
fn a_date_time_without_an_offset_is_unreadable_not_guessed() {
    let (data_type, _, reason) = unreadable("Local");
    assert_eq!(data_type, "IFCDATETIME");
    assert!(
        reason.contains("IFCDATETIME '2026-09-27T10:30:00' states no UTC offset"),
        "{reason}"
    );
}

#[test]
fn text_that_is_not_the_declared_date_form_is_an_invalid_value() {
    for name in ["BadDate", "NoDay", "BadDateTime", "FarZone"] {
        assert_eq!(
            resolve(name).err(),
            Some(PropertyResolutionError::InvalidValue),
            "{name}"
        );
    }
}

#[test]
fn a_logical_is_a_boolean_and_its_unknown_holds_no_value() {
    for (name, expected) in [
        ("True", PropertyValue::Boolean(true)),
        ("False", PropertyValue::Boolean(false)),
        // `.U.` states no truth value: present, with no value, as `$`.
        ("Unknown", PropertyValue::Null),
    ] {
        let (value, declared, _) = present("#1", "P", name);
        assert_eq!(value, expected, "{name}");
        assert_eq!(declared.as_deref(), Some("IFCLOGICAL"), "{name}");
    }
    // A list element is a value, so an unknown in a list is refused.
    assert_eq!(
        resolve("Unknowns").err(),
        Some(PropertyResolutionError::InexactEvidence)
    );
}

#[test]
fn a_date_attribute_keeps_its_time_zone() {
    let (value, _, _) = present("#17", ATTRIBUTE_SET, "EditionDate");
    let PropertyValue::Date(date) = value else {
        panic!("{value:?}");
    };
    assert_eq!(date.to_string(), "2022-01-01-05:00");
    assert_ne!(date, "2022-01-01".parse().unwrap());
}

#[test]
fn date_time_attributes_read_as_date_times() {
    let (value, _, _) = present("#14", ATTRIBUTE_SET, "CreationDate");
    assert_eq!(
        value,
        PropertyValue::DateTime("2026-09-27T08:00:00Z".parse().unwrap())
    );
    assert!(matches!(
        resolve_in("#14", ATTRIBUTE_SET, "StartTime"),
        Err(PropertyResolutionError::Incomplete(_))
    ));
}
