//! Property enumeration: every property whose set and name match, exactly,
//! or a refusal; an empty answer proves absence.
#![allow(missing_docs)]

use axioval_engine::{
    NameMatch, NamePattern, PropertyEnumeration, PropertyEnumerationRequest, PropertyRequest,
    PropertyResolution, PropertyResolutionError, PropertyResolutionServiceHandle,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{ObjectId, PropertyValue, SourceId};

/// Wall #1: an occurrence `Pset_WallCommon` (`IsExternal`, `Status`), a
/// `Pset_Custom` (`FooBar`, `FooBaz`, a complex `Layers`), a quantity set and,
/// through its type #30, an inherited `Pset_WallCommon` (`IsExternal`,
/// overridden, and `FireRating`). Wall #2 has two sets named `Twice` and a
/// reference value `Link` in `Pset_Links`. Wall #3 has no property.
const IFC4: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('n','t',(''),(''),'p','o','a');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#90=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#93=IFCUNITASSIGNMENT((#90));
#94=IFCPROJECT('0000000000000000000094',$,'P',$,$,$,$,$,#93);
#1=IFCWALL('0000000000000000000001',$,$,$,$,$,$,$,$);
#2=IFCWALL('0000000000000000000002',$,$,$,$,$,$,$,$);
#3=IFCWALL('0000000000000000000003',$,$,$,$,$,$,$,$);
#10=IFCPROPERTYSINGLEVALUE('IsExternal',$,IFCBOOLEAN(.T.),$);
#11=IFCPROPERTYENUMERATION('Status',(IFCLABEL('NEW'),IFCLABEL('EXISTING')),$);
#12=IFCPROPERTYENUMERATEDVALUE('Status',$,(IFCLABEL('NEW')),#11);
#13=IFCPROPERTYSET('0000000000000000000013',$,'Pset_WallCommon',$,(#10,#12));
#14=IFCRELDEFINESBYPROPERTIES('0000000000000000000014',$,$,$,(#1),#13);
#15=IFCPROPERTYSINGLEVALUE('FooBar',$,IFCLABEL('x'),$);
#16=IFCPROPERTYSINGLEVALUE('FooBaz',$,IFCLABEL('y'),$);
#17=IFCPROPERTYSINGLEVALUE('Inner',$,IFCLABEL('z'),$);
#18=IFCCOMPLEXPROPERTY('Layers',$,'layers',(#17));
#19=IFCPROPERTYSET('0000000000000000000019',$,'Pset_Custom',$,(#15,#16,#18));
#20=IFCRELDEFINESBYPROPERTIES('0000000000000000000020',$,$,$,(#1),#19);
#21=IFCQUANTITYLENGTH('Length',$,$,4000.,$);
#22=IFCELEMENTQUANTITY('0000000000000000000022',$,'Qto_WallBaseQuantities',$,$,(#21));
#23=IFCRELDEFINESBYPROPERTIES('0000000000000000000023',$,$,$,(#1),#22);
#30=IFCWALLTYPE('0000000000000000000030',$,'T',$,$,(#33),$,$,$,.STANDARD.);
#31=IFCPROPERTYSINGLEVALUE('IsExternal',$,IFCBOOLEAN(.F.),$);
#32=IFCPROPERTYSINGLEVALUE('FireRating',$,IFCLABEL('EI 60'),$);
#33=IFCPROPERTYSET('0000000000000000000033',$,'Pset_WallCommon',$,(#31,#32));
#34=IFCRELDEFINESBYTYPE('0000000000000000000034',$,$,$,(#1),#30);
#40=IFCPROPERTYSINGLEVALUE('A',$,IFCLABEL('a'),$);
#41=IFCPROPERTYSET('0000000000000000000041',$,'Twice',$,(#40));
#42=IFCPROPERTYSINGLEVALUE('B',$,IFCLABEL('b'),$);
#43=IFCPROPERTYSET('0000000000000000000043',$,'Twice',$,(#42));
#44=IFCRELDEFINESBYPROPERTIES('0000000000000000000044',$,$,$,(#2),#41);
#45=IFCRELDEFINESBYPROPERTIES('0000000000000000000045',$,$,$,(#2),#43);
#46=IFCPROPERTYREFERENCEVALUE('Link',$,$,#94);
#47=IFCPROPERTYSET('0000000000000000000047',$,'Pset_Links',$,(#46));
#48=IFCRELDEFINESBYPROPERTIES('0000000000000000000048',$,$,$,(#2),#47);
ENDSEC;
END-ISO-10303-21;
";

fn object(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), local).unwrap()
}

fn handle() -> PropertyResolutionServiceHandle {
    import_ifc_session("model.ifc", IFC4.as_bytes())
        .unwrap()
        .services()
        .get::<PropertyResolutionServiceHandle>()
        .unwrap()
        .clone()
}

fn pattern(regex: &str) -> NameMatch {
    NameMatch::Pattern(NamePattern::new(regex).unwrap())
}

fn exact(name: &str) -> NameMatch {
    NameMatch::Exact(name.into())
}

fn enumerate(
    local: &str,
    set: NameMatch,
    property: NameMatch,
) -> Result<PropertyEnumeration, PropertyResolutionError> {
    handle().enumerate(&PropertyEnumerationRequest::try_new(object(local), set, property).unwrap())
}

fn names(enumeration: &PropertyEnumeration) -> Vec<(String, String)> {
    enumeration
        .properties()
        .iter()
        .map(|property| (property.property_set.clone(), property.name.clone()))
        .collect()
}

fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    expected
        .iter()
        .map(|(set, name)| ((*set).to_owned(), (*name).to_owned()))
        .collect()
}

#[test]
fn a_pattern_selects_every_matching_property_sorted_by_set_and_name() {
    let found = enumerate("#1", exact("Pset_Custom"), pattern("Foo.*")).unwrap();
    assert_eq!(
        names(&found),
        pairs(&[("Pset_Custom", "FooBar"), ("Pset_Custom", "FooBaz")])
    );
    assert_eq!(
        found.properties()[0].value,
        PropertyValue::String("x".into())
    );
    // The unselected complex property does not refuse the answer.
    assert!(found.evidence().locator.contains("enumeration:"));
}

#[test]
fn inherited_properties_are_enumerated_and_occurrence_values_override_them() {
    let found = enumerate("#1", pattern("Pset_.*Common"), NameMatch::Any).unwrap();
    assert_eq!(
        names(&found),
        pairs(&[
            ("Pset_WallCommon", "FireRating"),
            ("Pset_WallCommon", "IsExternal"),
            ("Pset_WallCommon", "Status"),
        ])
    );
    assert_eq!(found.properties()[1].value, PropertyValue::Boolean(true));
    assert!(
        found.properties()[0]
            .evidence
            .as_ref()
            .unwrap()
            .locator
            .contains("type:")
    );
}

#[test]
fn quantities_are_properties_in_si() {
    let found = enumerate("#1", pattern("Qto_.*"), NameMatch::Any).unwrap();
    assert_eq!(
        names(&found),
        pairs(&[("Qto_WallBaseQuantities", "Length")])
    );
    assert!(matches!(
        found.properties()[0].value,
        PropertyValue::Quantity { value, .. } if (value - 4.0).abs() < 1e-12
    ));
}

#[test]
fn an_empty_enumeration_is_a_proof_that_agrees_with_resolution() {
    let found = enumerate("#1", pattern("Pset_Door.*"), NameMatch::Any).unwrap();
    assert!(found.properties().is_empty());
    assert!(
        enumerate("#3", NameMatch::Any, NameMatch::Any)
            .unwrap()
            .properties()
            .is_empty()
    );
    // One set and one name answer what resolution answers.
    for (set, name) in [
        ("Pset_WallCommon", "FireRating"),
        ("Pset_WallCommon", "IsExternal"),
        ("Pset_Custom", "Missing"),
    ] {
        let resolved = handle()
            .resolve(&PropertyRequest::try_new(object("#1"), Some(set.into()), name).unwrap())
            .unwrap();
        let listed = enumerate("#1", exact(set), exact(name)).unwrap();
        match resolved {
            PropertyResolution::Present(resolved) => {
                assert_eq!(listed.properties(), [resolved.property().clone()]);
            }
            PropertyResolution::Absent(_) => assert!(listed.properties().is_empty()),
        }
    }
}

#[test]
fn ambiguity_and_selected_values_the_ir_cannot_carry_are_refused() {
    // Two sets of one name are ambiguous once selected, not before.
    assert!(matches!(
        enumerate("#2", exact("Twice"), NameMatch::Any),
        Err(PropertyResolutionError::Conflicting(_))
    ));
    assert!(enumerate("#2", exact("Pset_Links"), NameMatch::Any).is_err());
    assert!(enumerate("#1", exact("Pset_Custom"), exact("Layers")).is_err());
    // A reserved set is never enumerated.
    assert_eq!(
        PropertyEnumerationRequest::try_new(
            object("#1"),
            exact("axioval:attributes"),
            NameMatch::Any
        ),
        Err(PropertyResolutionError::InvalidRequest)
    );
}

#[test]
fn a_property_set_holding_no_property_refuses_rather_than_proving_absence() {
    // `HasProperties` is `SET [1:?]`: an empty set is malformed, so neither
    // its properties' absence nor anything under its name is known.
    let model = IFC4.replace(
        "#48=IFCRELDEFINESBYPROPERTIES",
        "#50=IFCPROPERTYSET('0000000000000000000050',$,'Pset_Empty',$,());\n\
         #51=IFCRELDEFINESBYPROPERTIES('0000000000000000000051',$,$,$,(#3),#50);\n\
         #48=IFCRELDEFINESBYPROPERTIES",
    );
    let handle = import_ifc_session("model.ifc", model.as_bytes())
        .unwrap()
        .services()
        .get::<PropertyResolutionServiceHandle>()
        .unwrap()
        .clone();
    for set in [Some("Pset_Empty"), None] {
        let resolved = handle
            .resolve(&PropertyRequest::try_new(object("#3"), set.map(Into::into), "N").unwrap());
        assert!(
            matches!(resolved, Err(PropertyResolutionError::Incomplete(_))),
            "{set:?}: {resolved:?}"
        );
    }
    for set in [exact("Pset_Empty"), NameMatch::Any] {
        let listed = handle.enumerate(
            &PropertyEnumerationRequest::try_new(object("#3"), set, NameMatch::Any).unwrap(),
        );
        assert!(
            matches!(listed, Err(PropertyResolutionError::Incomplete(_))),
            "{listed:?}"
        );
    }
}
