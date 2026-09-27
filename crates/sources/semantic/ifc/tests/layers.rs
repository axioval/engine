//! Presentation layers resolve through shape representations and mapped items,
//! every one of an object's layers is listed, and a model without layers says so.
#![allow(missing_docs)]

use axioval_engine::{
    PropertyRequest, PropertyResolution, PropertyResolutionError, PropertyResolutionServiceHandle,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{ObjectId, PRESENTATION_LAYER, PRESENTATION_SET, PropertyValue, SourceId};

/// #1 is on A-WALL through its representation, #2 through an item, #3 through
/// the representation its mapped item maps in, #4 on two layers, #5 has no
/// shape at all, #6 a shape on no layer.
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
#6=IFCWALL('0000000000000000000006',$,'W6',$,$,$,#60,$,$);
#60=IFCPRODUCTDEFINITIONSHAPE($,$,(#61));
#61=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#90));
#80=IFCPRESENTATIONLAYERASSIGNMENT('A-WALL',$,(#11,#22,#41),$);
#81=IFCPRESENTATIONLAYERASSIGNMENT('A-DOOR',$,(#34),$);
#82=IFCPRESENTATIONLAYERASSIGNMENT('A-AXIS',$,(#42),$);
";

fn resolve(local: &str) -> Result<PropertyResolution, PropertyResolutionError> {
    resolve_in(DATA, local)
}

fn resolve_in(data: &str, local: &str) -> Result<PropertyResolution, PropertyResolutionError> {
    let bytes = format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
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

fn layers(local: &str) -> Option<Vec<String>> {
    match resolve(local).unwrap() {
        PropertyResolution::Present(resolved) => match &resolved.property().value {
            PropertyValue::List(names) => Some(
                names
                    .iter()
                    .map(|name| match name {
                        PropertyValue::String(name) => name.clone(),
                        other => panic!("not a layer name: {other:?}"),
                    })
                    .collect(),
            ),
            other => panic!("not a layer list: {other:?}"),
        },
        PropertyResolution::Absent(_) => None,
    }
}

#[test]
fn a_layer_on_the_representation_or_an_item_is_the_objects_layer() {
    assert_eq!(layers("#1").unwrap(), ["A-WALL"]);
    assert_eq!(layers("#2").unwrap(), ["A-WALL"]);
    let PropertyResolution::Present(resolved) = resolve("#1").unwrap() else {
        panic!("expected a layer");
    };
    let locator = &resolved.property().evidence.as_ref().unwrap().locator;
    assert!(locator.ends_with("layer:#1:#80"), "{locator}");
}

#[test]
fn a_mapped_items_layer_reaches_the_occurrence() {
    assert_eq!(layers("#3").unwrap(), ["A-DOOR"]);
}

#[test]
fn every_layer_of_an_object_is_listed_by_name_with_its_assignment() {
    assert_eq!(layers("#4").unwrap(), ["A-AXIS", "A-WALL"]);
    let PropertyResolution::Present(resolved) = resolve("#4").unwrap() else {
        panic!("expected layers");
    };
    let locator = &resolved.property().evidence.as_ref().unwrap().locator;
    assert!(locator.ends_with("layer:#4:#82,#80"), "{locator}");
}

#[test]
fn no_layer_in_a_layered_model_is_absence() {
    assert_eq!(layers("#5"), None);
    assert_eq!(layers("#6"), None);
}

#[test]
fn a_model_without_layer_assignments_records_no_layers() {
    let unlayered: String = DATA
        .lines()
        .filter(|line| !line.contains("IFCPRESENTATIONLAYERASSIGNMENT"))
        .flat_map(|line| [line, "\n"])
        .collect();
    for local in ["#1", "#5"] {
        assert!(matches!(
            resolve_in(&unlayered, local),
            Err(PropertyResolutionError::NotRecorded(message))
                if message.contains("no presentation layers") && !message.contains(local)
        ));
    }
}
