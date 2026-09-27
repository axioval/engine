//! Materials resolve through `IfcRelAssociatesMaterial`, on the occurrence or
//! its type, into the reserved material set.
#![allow(missing_docs)]

use axioval_engine::{
    PropertyRequest, PropertyResolution, PropertyResolutionError, PropertyResolutionServiceHandle,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{MATERIAL_SET, ObjectId, PropertyValue, QuantityDimension, SourceId};

/// Millimetres. #1 takes the layer set of its type #40; #2 has the same type
/// and its own layer set usage; #3 is one material; #4 a constituent set; #5
/// a profile set; #6 has no material and no type; #7 a material list; #8 two
/// direct assignments.
const DATA: &str = "\
#90=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#91=IFCUNITASSIGNMENT((#90));
#92=IFCPROJECT('000000000000000000000P',$,'P',$,$,$,$,$,#91);
#20=IFCMATERIAL('Concrete',$,'Structure');
#21=IFCMATERIAL('Mineral wool',$,$);
#22=IFCMATERIAL('Gypsum',$,$);
#23=IFCMATERIAL('Glass',$,$);
#30=IFCMATERIALLAYER(#22,12.5,$,'Board',$,'Finish',$);
#31=IFCMATERIALLAYER(#21,100.,.F.,$,$,$,$);
#32=IFCMATERIALLAYER(#20,200.,$,'Core',$,'LoadBearing',$);
#33=IFCMATERIALLAYER($,20.,.T.,'Air',$,$,$);
#35=IFCMATERIALLAYERSET((#30,#31,#32,#33),'WT-01',$);
#36=IFCMATERIALLAYERSETUSAGE(#35,.AXIS2.,.POSITIVE.,0.,$);
#40=IFCWALLTYPE('0000000000000000000040',$,'WT-01',$,$,$,$,$,$,.STANDARD.);
#41=IFCRELASSOCIATESMATERIAL('0000000000000000000041',$,$,$,(#40),#35);
#42=IFCRELDEFINESBYTYPE('0000000000000000000042',$,$,$,(#1,#2),#40);
#43=IFCRELASSOCIATESMATERIAL('0000000000000000000043',$,$,$,(#2),#36);
#1=IFCWALL('0000000000000000000001',$,'W1',$,$,$,$,$,$);
#2=IFCWALL('0000000000000000000002',$,'W2',$,$,$,$,$,$);
#3=IFCSLAB('0000000000000000000003',$,'S1',$,$,$,$,$,$);
#44=IFCRELASSOCIATESMATERIAL('0000000000000000000044',$,$,$,(#3),#20);
#50=IFCMATERIALCONSTITUENT('Frame',$,#22,0.3,'Frame');
#51=IFCMATERIALCONSTITUENT('Glazing',$,#23,0.7,$);
#52=IFCMATERIALCONSTITUENTSET('Window',$,(#50,#51));
#4=IFCBUILDINGELEMENTPROXY('0000000000000000000004',$,'X1',$,$,$,$,$,$);
#45=IFCRELASSOCIATESMATERIAL('0000000000000000000045',$,$,$,(#4),#52);
#60=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,100.,200.);
#61=IFCMATERIALPROFILE('Web',$,#20,#60,$,$);
#62=IFCMATERIALPROFILESET('B-100',$,(#61),$);
#5=IFCBEAM('0000000000000000000005',$,'B1',$,$,$,$,$,$);
#46=IFCRELASSOCIATESMATERIAL('0000000000000000000046',$,$,$,(#5),#62);
#6=IFCWALL('0000000000000000000006',$,'W6',$,$,$,$,$,$);
#63=IFCMATERIALLIST((#20,#23));
#7=IFCCOLUMN('0000000000000000000007',$,'C1',$,$,$,$,$,$);
#47=IFCRELASSOCIATESMATERIAL('0000000000000000000047',$,$,$,(#7),#63);
#8=IFCWALL('0000000000000000000008',$,'W8',$,$,$,$,$,$);
#48=IFCRELASSOCIATESMATERIAL('0000000000000000000048',$,$,$,(#8),#20);
#49=IFCRELASSOCIATESMATERIAL('0000000000000000000049',$,$,$,(#8),#22);
";

fn resolve_in(
    schema: &str,
    data: &str,
    local: &str,
    name: &str,
) -> Result<PropertyResolution, PropertyResolutionError> {
    let bytes = format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('{schema}'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
    );
    let session = import_ifc_session("model.ifc", bytes.as_bytes()).unwrap();
    let object = ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), local).unwrap();
    let request = PropertyRequest::try_new(object, Some(MATERIAL_SET.into()), name).unwrap();
    session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap()
        .resolve(&request)
}

fn resolve(local: &str, name: &str) -> Result<PropertyResolution, PropertyResolutionError> {
    resolve_in("IFC4", DATA, local, name)
}

fn value(local: &str, name: &str) -> Option<PropertyValue> {
    match resolve(local, name).unwrap() {
        PropertyResolution::Present(resolved) => Some(resolved.property().value.clone()),
        PropertyResolution::Absent(_) => None,
    }
}

fn text(local: &str, name: &str) -> Option<String> {
    value(local, name).map(|value| match value {
        PropertyValue::String(text) => text,
        other => panic!("{name} is not text: {other:?}"),
    })
}

fn metres(local: &str, name: &str) -> f64 {
    match value(local, name) {
        Some(PropertyValue::Quantity {
            value,
            dimension: QuantityDimension::Length,
        }) => value,
        other => panic!("{name} is not a length: {other:?}"),
    }
}

fn locator(local: &str, name: &str) -> String {
    let PropertyResolution::Present(resolved) = resolve(local, name).unwrap() else {
        panic!("{name} is absent");
    };
    resolved
        .property()
        .evidence
        .as_ref()
        .unwrap()
        .locator
        .clone()
}

#[test]
fn a_walls_layer_set_is_read_through_its_type() {
    assert_eq!(text("#1", "Kind").as_deref(), Some("layer-set"));
    assert_eq!(text("#1", "Name").as_deref(), Some("WT-01"));
    assert_eq!(value("#1", "Count"), Some(PropertyValue::Integer(4)));
    assert!((metres("#1", "TotalThickness") - 0.3325).abs() < 1e-12);
    assert!((metres("#1", "Layer1.Thickness") - 0.0125).abs() < 1e-12);
    assert!((metres("#1", "Layer3.Thickness") - 0.2).abs() < 1e-12);
    assert_eq!(text("#1", "Layer1.Material").as_deref(), Some("Gypsum"));
    assert_eq!(text("#1", "Layer1.Name").as_deref(), Some("Board"));
    assert_eq!(text("#1", "Layer1.Category").as_deref(), Some("Finish"));
    assert_eq!(text("#1", "Layer3.Material").as_deref(), Some("Concrete"));
    let found = locator("#1", "TotalThickness");
    assert!(found.ends_with("material:#1:type:#40:#41:#35"), "{found}");
    let found = locator("#1", "Layer3.Material");
    assert!(
        found.ends_with("material:#1:type:#40:#41:#32/#20"),
        "{found}"
    );
}

#[test]
fn an_occurrence_usage_wins_over_its_type() {
    assert!((metres("#2", "TotalThickness") - 0.3325).abs() < 1e-12);
    let found = locator("#2", "Layer2.Material");
    assert!(
        found.ends_with("material:#2:occurrence:#43:usage:#36:#31/#21"),
        "{found}"
    );
}

#[test]
fn unstated_members_and_names_outside_the_set_are_absent() {
    // The air layer has no material; there is no fifth layer.
    assert_eq!(text("#1", "Layer4.Name").as_deref(), Some("Air"));
    assert_eq!(value("#1", "Layer4.Material"), None);
    assert_eq!(value("#1", "Layer5.Thickness"), None);
    assert_eq!(value("#1", "Layer2.Name"), None);
    assert_eq!(value("#1", "Category"), None);
    assert_eq!(value("#1", "Thickness"), None);
}

#[test]
fn a_single_material_is_named_ignoring_property_case() {
    assert_eq!(text("#3", "kind").as_deref(), Some("material"));
    assert_eq!(text("#3", "NAME").as_deref(), Some("Concrete"));
    assert_eq!(text("#3", "Category").as_deref(), Some("Structure"));
    assert_eq!(value("#3", "Count"), None);
    assert_eq!(value("#3", "TotalThickness"), None);
    assert!(locator("#3", "Name").ends_with("material:#3:occurrence:#44:#20"));
}

#[test]
fn constituent_profile_and_list_members_are_numbered_in_order() {
    assert_eq!(text("#4", "Kind").as_deref(), Some("constituent-set"));
    assert_eq!(text("#4", "Name").as_deref(), Some("Window"));
    assert_eq!(
        text("#4", "Constituent2.Material").as_deref(),
        Some("Glass")
    );
    assert_eq!(text("#4", "Constituent1.Name").as_deref(), Some("Frame"));
    assert_eq!(
        value("#4", "Constituent1.Fraction"),
        Some(PropertyValue::Decimal(0.3))
    );

    assert_eq!(text("#5", "Kind").as_deref(), Some("profile-set"));
    assert_eq!(text("#5", "Name").as_deref(), Some("B-100"));
    assert_eq!(text("#5", "Profile1.Name").as_deref(), Some("Web"));
    assert_eq!(text("#5", "Profile1.Material").as_deref(), Some("Concrete"));

    assert_eq!(text("#7", "Kind").as_deref(), Some("list"));
    assert_eq!(value("#7", "Count"), Some(PropertyValue::Integer(2)));
    assert_eq!(text("#7", "Material2.Name").as_deref(), Some("Glass"));
    assert_eq!(value("#7", "Name"), None);
}

#[test]
fn an_object_without_material_has_exactly_none() {
    for name in ["Kind", "Name", "TotalThickness", "Layer1.Material"] {
        let PropertyResolution::Absent(absence) = resolve("#6", name).unwrap() else {
            panic!("{name} is present");
        };
        assert!(absence.evidence().exact);
    }
}

#[test]
fn two_direct_assignments_conflict() {
    assert!(matches!(
        resolve("#8", "Name"),
        Err(PropertyResolutionError::Conflicting(_))
    ));
}

#[test]
fn a_layer_with_negative_thickness_is_refused() {
    let data = DATA.replace(
        "#31=IFCMATERIALLAYER(#21,100.,",
        "#31=IFCMATERIALLAYER(#21,-1.,",
    );
    for name in ["Kind", "Layer1.Material"] {
        assert!(matches!(
            resolve_in("IFC4", &data, "#1", name),
            Err(PropertyResolutionError::Incomplete(_))
        ));
    }
}

#[test]
fn ifc2x3_materials_are_refused_until_upstream_binds_the_release() {
    let data = "\
#20=IFCMATERIAL('Concrete');
#1=IFCWALL('0000000000000000000001',$,'W1',$,$,$,$,$);
#44=IFCRELASSOCIATESMATERIAL('0000000000000000000044',$,$,$,(#1),#20);
";
    assert!(matches!(
        resolve_in("IFC2X3", data, "#1", "Name"),
        Err(PropertyResolutionError::Unavailable(message)) if message.contains("IFC2X3")
    ));
}
