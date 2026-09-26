//! Presentation layers resolve through shape representations and mapped items.
#![allow(missing_docs)]

use axioval_engine::{
    PropertyRequest, PropertyResolution, PropertyResolutionError, PropertyResolutionServiceHandle,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{ObjectId, PRESENTATION_LAYER, PRESENTATION_SET, PropertyValue, SourceId};

/// #1 is on A-WALL through its representation, #2 through an item, #3 through
/// the representation its mapped item maps in, #4 on two layers, #5 has no
/// shape at all.
const DATA: &str = "\
#90=IFCCARTESIANPOINT((0.,0.,0.));
#91=IFCAXIS2PLACEMENT3D(#90,$,$);
#92=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#91,$);
#1=IFCWALL('0000000000000000000001',$,'W1',$,$,$,#10,$,$);
#10=IFCPRODUCTDEFINITIONSHAPE($,$,(#11));
#11=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#90));
#2=IFCWALL('0000000000000000000002',$,'W2',$,$,$,#20,$,$);
#20=IFCPRODUCTDEFINITIONSHAPE($,$,(#21));
#21=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#22));
#22=IFCCARTESIANPOINT((1.,0.,0.));
#3=IFCDOOR('0000000000000000000003',$,'D1',$,$,$,#30,$,$,$,$,$,$);
#30=IFCPRODUCTDEFINITIONSHAPE($,$,(#31));
#31=IFCSHAPEREPRESENTATION(#92,'Body','MappedRepresentation',(#32));
#32=IFCMAPPEDITEM(#33,#35);
#33=IFCREPRESENTATIONMAP(#91,#34);
#34=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#90));
#35=IFCCARTESIANTRANSFORMATIONOPERATOR3D($,$,#90,$,$);
#4=IFCWALL('0000000000000000000004',$,'W4',$,$,$,#40,$,$);
#40=IFCPRODUCTDEFINITIONSHAPE($,$,(#41,#42));
#41=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#90));
#42=IFCSHAPEREPRESENTATION(#92,'Axis','Curve2D',(#90));
#5=IFCWALL('0000000000000000000005',$,'W5',$,$,$,$,$,$);
#80=IFCPRESENTATIONLAYERASSIGNMENT('A-WALL',$,(#11,#22,#41),$);
#81=IFCPRESENTATIONLAYERASSIGNMENT('A-DOOR',$,(#34),$);
#82=IFCPRESENTATIONLAYERASSIGNMENT('A-AXIS',$,(#42),$);
";

fn resolve(local: &str) -> Result<PropertyResolution, PropertyResolutionError> {
    let bytes = format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n{DATA}ENDSEC;\nEND-ISO-10303-21;\n"
    );
    let session = import_ifc_session("model.ifc", bytes.as_bytes()).unwrap();
    let object = ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), local).unwrap();
    let request =
        PropertyRequest::try_new(object, Some(PRESENTATION_SET.into()), PRESENTATION_LAYER)
            .unwrap();
    session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap()
        .resolve(&request)
}

fn layer(local: &str) -> Option<String> {
    match resolve(local).unwrap() {
        PropertyResolution::Present(resolved) => match &resolved.property().value {
            PropertyValue::String(name) => Some(name.clone()),
            other => panic!("not a layer name: {other:?}"),
        },
        PropertyResolution::Absent(_) => None,
    }
}

#[test]
fn a_layer_on_the_representation_or_an_item_is_the_objects_layer() {
    assert_eq!(layer("#1").as_deref(), Some("A-WALL"));
    assert_eq!(layer("#2").as_deref(), Some("A-WALL"));
    let PropertyResolution::Present(resolved) = resolve("#1").unwrap() else {
        panic!("expected a layer");
    };
    let locator = &resolved.property().evidence.as_ref().unwrap().locator;
    assert!(locator.ends_with("layer:#1:#80"), "{locator}");
}

#[test]
fn a_mapped_items_layer_reaches_the_occurrence() {
    assert_eq!(layer("#3").as_deref(), Some("A-DOOR"));
}

#[test]
fn several_layers_conflict_and_no_shape_is_absence() {
    assert!(matches!(
        resolve("#4"),
        Err(PropertyResolutionError::Conflicting(message)) if message.contains("A-AXIS, A-WALL")
    ));
    assert_eq!(layer("#5"), None);
}
