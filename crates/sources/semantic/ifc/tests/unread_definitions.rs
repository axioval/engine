//! Absence is proven only over definitions the exact resolver reads.
//!
//! `ifc-properties` resolves `IfcPropertySet` members and skips every other
//! `IfcPropertySetDefinition`: quantity sets and predefined property sets. Its
//! `Absent` therefore means "in no property set", and must not become exact
//! absence evidence while a skipped definition could hold the property.

use axioval_engine::{
    PropertyRequest, PropertyResolution, PropertyResolutionError, PropertyResolutionServiceHandle,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{ObjectId, SourceId};

/// A wall with a quantity set (one quantity nested in a complex quantity)
/// and a property set, and a door with a predefined lining property set.
const IFC4: &[u8] = b"ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCWALL('0000000000000000000001',$,$,$,$,$,$,$,$);
#2=IFCQUANTITYLENGTH('Foo',$,$,42.,$);
#3=IFCPHYSICALCOMPLEXQUANTITY('Layer',$,(#4),'layer',$,$);
#4=IFCQUANTITYLENGTH('Inner',$,$,1.,$);
#5=IFCELEMENTQUANTITY('0000000000000000000002',$,'Foo_Bar',$,$,(#2,#3));
#6=IFCRELDEFINESBYPROPERTIES('0000000000000000000003',$,$,$,(#1),#5);
#7=IFCPROPERTYSINGLEVALUE('Other',$,IFCLABEL('x'),$);
#8=IFCPROPERTYSET('0000000000000000000004',$,'Pset_Test',$,(#7));
#9=IFCRELDEFINESBYPROPERTIES('0000000000000000000005',$,$,$,(#1),#8);
#10=IFCDOOR('0000000000000000000006',$,$,$,$,$,$,$,$,$,$,$,$);
#11=IFCDOORLININGPROPERTIES('0000000000000000000007',$,'Lining',$,0.1,$,$,$,$,$,$,$,$,$,$,$,$);
#12=IFCRELDEFINESBYPROPERTIES('0000000000000000000008',$,$,$,(#10),#11);
ENDSEC;
END-ISO-10303-21;
";

fn resolve(
    object: &str,
    set: Option<&str>,
    name: &str,
) -> Result<PropertyResolution, PropertyResolutionError> {
    let session = import_ifc_session("model.ifc", IFC4).unwrap();
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

#[test]
fn a_named_set_that_is_not_a_property_set_proves_nothing_absent() {
    for (object, set, name) in [
        ("#1", "Foo_Bar", "Foo"),
        ("#1", "Foo_Bar", "Anything"),
        ("#10", "Lining", "LiningDepth"),
    ] {
        let result = resolve(object, Some(set), name);
        assert!(
            matches!(result, Err(PropertyResolutionError::Incomplete(ref message)) if message.contains(set)),
            "{object} {set}.{name}: {result:?}"
        );
    }
}

#[test]
fn an_unqualified_name_held_by_an_unread_definition_proves_nothing_absent() {
    // A quantity, one nested in a complex quantity, the complex quantity
    // itself, and a predefined set's attribute.
    for name in ["Foo", "Inner", "Layer", "LiningDepth"] {
        let result = resolve("#1", None, name);
        assert!(
            matches!(result, Err(PropertyResolutionError::Incomplete(_))),
            "{name}: {result:?}"
        );
    }
}

#[test]
fn absence_outside_unread_definitions_stays_exact() {
    for (set, name) in [
        (Some("Pset_Test"), "Foo"),
        (None, "Nothing"),
        (Some("Pset_Other"), "Foo"),
    ] {
        let result = resolve("#1", set, name);
        assert!(
            matches!(result, Ok(PropertyResolution::Absent(_))),
            "{set:?}.{name}: {result:?}"
        );
    }
    assert!(matches!(
        resolve("#1", Some("Pset_Test"), "Other"),
        Ok(PropertyResolution::Present(_))
    ));
}

#[test]
fn ifc2x3_quantity_sets_are_read_with_ifc2x3_tables() {
    const IFC2X3: &[u8] = b"ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC2X3'));
ENDSEC;
DATA;
#1=IFCWALL('0000000000000000000001',$,$,$,$,$,$,$);
#2=IFCQUANTITYLENGTH('Width',$,$,0.3);
#3=IFCELEMENTQUANTITY('0000000000000000000002',$,'BaseQuantities',$,$,(#2));
#4=IFCRELDEFINESBYPROPERTIES('0000000000000000000003',$,$,$,(#1),#3);
ENDSEC;
END-ISO-10303-21;
";
    let session = import_ifc_session("model.ifc", IFC2X3).unwrap();
    let service = session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap();
    for (set, name) in [(Some("BaseQuantities"), "Width"), (None, "Width")] {
        let request = PropertyRequest::try_new(
            ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), "#1").unwrap(),
            set.map(ToOwned::to_owned),
            name,
        )
        .unwrap();
        let result = service.resolve(&request);
        assert!(
            matches!(result, Err(PropertyResolutionError::Incomplete(_))),
            "{set:?}.{name}: {result:?}"
        );
    }
}
