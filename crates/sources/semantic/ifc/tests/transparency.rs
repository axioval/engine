//! Surface transparency resolves from the styles of an object's body items,
//! falls back to its material's styles for unstyled items, and lists every
//! distinct value; an object without a styled surface is exactly absent.
#![allow(missing_docs)]

use axioval_engine::{
    PropertyRequest, PropertyResolution, PropertyResolutionError, PropertyResolutionServiceHandle,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{ObjectId, PRESENTATION_SET, PRESENTATION_TRANSPARENCY, PropertyValue, SourceId};
use ifc_model::{Codec, EntityId};
use ifc_step::StepCodec;
use ifc_style::StyleView;

/// Styles: #50 glass (0.7), #51 a rendering without transparency (opaque),
/// #52 frosted (0.6), #53 a curve style, #54 only an external definition,
/// #55 tinted (0.2), #56 matte material (0.9) for #20.
///
/// #1 is styled glass; #2 renders opaque; #3 has a glass and a frosted item
/// and an `Axis` styled by a curve; #4 maps in a styled representation; #5
/// has an unstyled item and the material #20; #6 styles its item tinted over
/// the material #20; #7 has no shape; #8 an unstyled item and no material;
/// #9 an item with two styled items; #10 an item styled only externally;
/// #11 an item on a styled layer; #12 an item styled by the curve only, so
/// its material draws it.
const DATA: &str = "\
#90=IFCCARTESIANPOINT((0.,0.,0.));
#91=IFCAXIS2PLACEMENT3D(#90,$,$);
#92=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#91,$);
#40=IFCCOLOURRGB($,0.5,0.5,0.5);
#41=IFCSURFACESTYLESHADING(#40,0.7);
#42=IFCSURFACESTYLERENDERING(#40,$,$,$,$,$,$,$,.NOTDEFINED.);
#43=IFCSURFACESTYLESHADING(#40,0.6);
#44=IFCSURFACESTYLESHADING(#40,0.2);
#45=IFCSURFACESTYLESHADING(#40,0.9);
#46=IFCEXTERNALLYDEFINEDSURFACESTYLE('lib.mat','Glass-1',$);
#50=IFCSURFACESTYLE('Glass',.BOTH.,(#41));
#51=IFCSURFACESTYLE('Painted',.POSITIVE.,(#42));
#52=IFCSURFACESTYLE('Frosted',.BOTH.,(#43));
#53=IFCCURVESTYLE('Axis',$,$,#40,$);
#54=IFCSURFACESTYLE('External',.BOTH.,(#46));
#55=IFCSURFACESTYLE('Tinted',.BOTH.,(#44));
#56=IFCSURFACESTYLE('Matte',.BOTH.,(#45));
#1=IFCWINDOW('0000000000000000000001',$,'G1',$,$,$,#10,$,$,$,$,$,$);
#10=IFCPRODUCTDEFINITIONSHAPE($,$,(#11));
#11=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#12));
#12=IFCCARTESIANPOINT((1.,0.,0.));
#13=IFCSTYLEDITEM(#12,(#50),$);
#2=IFCWALL('0000000000000000000002',$,'W2',$,$,$,#20,$,$);
#20=IFCPRODUCTDEFINITIONSHAPE($,$,(#21));
#21=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#22));
#22=IFCCARTESIANPOINT((2.,0.,0.));
#23=IFCSTYLEDITEM(#22,(#51),$);
#3=IFCWINDOW('0000000000000000000003',$,'G3',$,$,$,#30,$,$,$,$,$,$);
#30=IFCPRODUCTDEFINITIONSHAPE($,$,(#31,#36));
#31=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#32,#33));
#32=IFCCARTESIANPOINT((3.,0.,0.));
#33=IFCCARTESIANPOINT((3.,1.,0.));
#34=IFCSTYLEDITEM(#32,(#50),$);
#35=IFCSTYLEDITEM(#33,(#52),$);
#36=IFCSHAPEREPRESENTATION(#92,'Axis','Curve2D',(#37));
#37=IFCCARTESIANPOINT((3.,2.,0.));
#38=IFCSTYLEDITEM(#37,(#53),$);
#4=IFCDOOR('0000000000000000000004',$,'D4',$,$,$,#60,$,$,$,$,$,$);
#60=IFCPRODUCTDEFINITIONSHAPE($,$,(#61));
#61=IFCSHAPEREPRESENTATION(#92,'Body','MappedRepresentation',(#62));
#62=IFCMAPPEDITEM(#63,#66);
#63=IFCREPRESENTATIONMAP(#91,#64);
#64=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#65));
#65=IFCCARTESIANPOINT((4.,0.,0.));
#66=IFCCARTESIANTRANSFORMATIONOPERATOR3D($,$,#90,$,$);
#67=IFCSTYLEDITEM(#65,(#52),$);
#200=IFCMATERIAL('Matte',$,$);
#201=IFCSTYLEDITEM($,(#56),$);
#202=IFCSTYLEDREPRESENTATION(#92,'Style','Material',(#201));
#203=IFCMATERIALDEFINITIONREPRESENTATION($,$,(#202),#200);
#204=IFCRELASSOCIATESMATERIAL('0000000000000000000204',$,$,$,(#5,#6,#12000),#200);
#5=IFCWALL('0000000000000000000005',$,'W5',$,$,$,#70,$,$);
#70=IFCPRODUCTDEFINITIONSHAPE($,$,(#71));
#71=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#72));
#72=IFCCARTESIANPOINT((5.,0.,0.));
#6=IFCWALL('0000000000000000000006',$,'W6',$,$,$,#80,$,$);
#80=IFCPRODUCTDEFINITIONSHAPE($,$,(#81));
#81=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#82));
#82=IFCCARTESIANPOINT((6.,0.,0.));
#83=IFCSTYLEDITEM(#82,(#55),$);
#7=IFCWALL('0000000000000000000007',$,'W7',$,$,$,$,$,$);
#8=IFCWALL('0000000000000000000008',$,'W8',$,$,$,#100,$,$);
#100=IFCPRODUCTDEFINITIONSHAPE($,$,(#101));
#101=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#102));
#102=IFCCARTESIANPOINT((8.,0.,0.));
#9=IFCWALL('0000000000000000000009',$,'W9',$,$,$,#110,$,$);
#110=IFCPRODUCTDEFINITIONSHAPE($,$,(#111));
#111=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#112));
#112=IFCCARTESIANPOINT((9.,0.,0.));
#113=IFCSTYLEDITEM(#112,(#50),$);
#114=IFCSTYLEDITEM(#112,(#51),$);
#10000=IFCWALL('0000000000000000010000',$,'W10',$,$,$,#10001,$,$);
#10001=IFCPRODUCTDEFINITIONSHAPE($,$,(#10002));
#10002=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#10003));
#10003=IFCCARTESIANPOINT((10.,0.,0.));
#10004=IFCSTYLEDITEM(#10003,(#54),$);
#11000=IFCWALL('0000000000000000011000',$,'W11',$,$,$,#11001,$,$);
#11001=IFCPRODUCTDEFINITIONSHAPE($,$,(#11002));
#11002=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#11003));
#11003=IFCCARTESIANPOINT((11.,0.,0.));
#11004=IFCPRESENTATIONLAYERWITHSTYLE('Glazing',$,(#11003),$,.T.,.F.,.F.,(#50));
#12000=IFCWALL('0000000000000000012000',$,'W12',$,$,$,#12001,$,$);
#12001=IFCPRODUCTDEFINITIONSHAPE($,$,(#12002));
#12002=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#12003));
#12003=IFCCARTESIANPOINT((12.,0.,0.));
#12004=IFCSTYLEDITEM(#12003,(#53),$);
";

fn step(schema: &str, data: &str) -> String {
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('{schema}'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

fn resolve_in(
    schema: &str,
    data: &str,
    local: &str,
) -> Result<PropertyResolution, PropertyResolutionError> {
    let session = import_ifc_session("model.ifc", step(schema, data).as_bytes()).unwrap();
    let object = ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), local).unwrap();
    let request = PropertyRequest::try_new(
        object,
        Some(PRESENTATION_SET.into()),
        PRESENTATION_TRANSPARENCY,
    )
    .unwrap();
    session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap()
        .resolve(&request)
}

fn resolve(local: &str) -> Result<PropertyResolution, PropertyResolutionError> {
    resolve_in("IFC4", DATA, local)
}

fn values_of(resolution: PropertyResolution) -> Option<Vec<f64>> {
    match resolution {
        PropertyResolution::Present(resolved) => match &resolved.property().value {
            PropertyValue::List(values) => Some(
                values
                    .iter()
                    .map(|value| match value {
                        PropertyValue::Decimal(value) => *value,
                        other => panic!("not a transparency: {other:?}"),
                    })
                    .collect(),
            ),
            other => panic!("not a transparency list: {other:?}"),
        },
        PropertyResolution::Absent(_) => None,
    }
}

fn transparency(local: &str) -> Option<Vec<f64>> {
    values_of(resolve(local).unwrap())
}

fn locator(local: &str) -> String {
    let PropertyResolution::Present(resolved) = resolve(local).unwrap() else {
        panic!("expected a transparency for {local}");
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
fn a_styled_item_states_its_objects_transparency() {
    assert_eq!(transparency("#1").unwrap(), [0.7]);
    let locator = locator("#1");
    assert!(locator.ends_with("transparency:#1:#50"), "{locator}");
}

#[test]
fn an_unset_transparency_is_opaque() {
    assert_eq!(transparency("#2").unwrap(), [0.0]);
}

#[test]
fn every_distinct_surface_transparency_is_listed_ascending() {
    // The curve style of the axis says nothing about a surface.
    assert_eq!(transparency("#3").unwrap(), [0.6, 0.7]);
    let locator = locator("#3");
    assert!(locator.ends_with("transparency:#3:#50,#52"), "{locator}");
}

#[test]
fn a_mapped_representations_styles_reach_the_occurrence() {
    assert_eq!(transparency("#4").unwrap(), [0.6]);
}

#[test]
fn an_unstyled_item_is_drawn_with_its_materials_style() {
    assert_eq!(transparency("#5").unwrap(), [0.9]);
    let locator = locator("#5");
    assert!(
        locator.ends_with("transparency:#5:material:#56"),
        "{locator}"
    );
    // An item styled by a curve style only has no surface style of its own.
    assert_eq!(transparency("#12000").unwrap(), [0.9]);
}

#[test]
fn an_items_own_style_wins_over_its_materials() {
    assert_eq!(transparency("#6").unwrap(), [0.2]);
}

#[test]
fn a_styled_layer_styles_its_items() {
    assert_eq!(transparency("#11000").unwrap(), [0.7]);
}

#[test]
fn an_object_without_a_styled_surface_is_absent() {
    assert_eq!(transparency("#7"), None);
    assert_eq!(transparency("#8"), None);
}

#[test]
fn two_styled_items_on_one_item_conflict() {
    assert!(matches!(
        resolve("#9"),
        Err(PropertyResolutionError::Conflicting(message)) if message.contains("#112")
    ));
}

#[test]
fn a_surface_style_without_shading_is_refused() {
    assert!(matches!(
        resolve("#10000"),
        Err(PropertyResolutionError::Incomplete(message))
            if message.contains("#54") && message.contains("no transparency")
    ));
}

/// The adapter's cascade agrees with `ifc-style`'s own item resolution on
/// every singly styled body item of the fixture.
#[test]
fn item_styles_agree_with_ifc_style_resolution() {
    let model = StepCodec.read_bytes(step("IFC4", DATA).as_bytes()).unwrap();
    let schema = ifc_schema::ifc4();
    let view = StyleView::new(&model, schema);
    for (object, item) in [
        ("#1", 12),
        ("#2", 22),
        ("#6", 82),
        ("#11000", 11003),
        ("#8", 102),
    ] {
        let resolved = view.resolve_item_style(EntityId(item)).unwrap();
        let expected: Vec<f64> = resolved
            .effective_styles()
            .iter()
            .filter(|style| schema.is_a(&model.get(**style).unwrap().type_name, "IfcSurfaceStyle"))
            .map(|style| {
                let shading = view.surface_style(*style).unwrap().elements().unwrap()[0];
                view.surface_style_shading(shading)
                    .unwrap()
                    .transparency()
                    .unwrap()
                    .unwrap_or(0.0)
            })
            .collect();
        let actual = transparency(object).unwrap_or_default();
        assert_eq!(actual, expected, "{object}");
    }
    assert!(view.resolve_item_style(EntityId(112)).is_err());
}

/// IFC2X3: styles through `IfcPresentationStyleAssignment`, transparency on
/// the rendering only; #1 renders at 0.4, #2 shades without a transparency
/// slot, #3 has an unstyled item and a styled material.
const IFC2X3: &str = "\
#90=IFCCARTESIANPOINT((0.,0.,0.));
#91=IFCAXIS2PLACEMENT3D(#90,$,$);
#92=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#91,$);
#40=IFCCOLOURRGB($,0.5,0.5,0.5);
#41=IFCSURFACESTYLERENDERING(#40,0.4,$,$,$,$,$,$,.FLAT.);
#42=IFCSURFACESTYLESHADING(#40);
#50=IFCSURFACESTYLE('Glass',.BOTH.,(#41));
#51=IFCSURFACESTYLE('Paint',.BOTH.,(#42));
#52=IFCPRESENTATIONSTYLEASSIGNMENT((#50));
#53=IFCPRESENTATIONSTYLEASSIGNMENT((#51));
#1=IFCWALL('0000000000000000000001',$,$,$,$,$,#10,$);
#10=IFCPRODUCTDEFINITIONSHAPE($,$,(#11));
#11=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#12));
#12=IFCCARTESIANPOINT((1.,0.,0.));
#13=IFCSTYLEDITEM(#12,(#52),$);
#2=IFCWALL('0000000000000000000002',$,$,$,$,$,#20,$);
#20=IFCPRODUCTDEFINITIONSHAPE($,$,(#21));
#21=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#22));
#22=IFCCARTESIANPOINT((2.,0.,0.));
#23=IFCSTYLEDITEM(#22,(#53),$);
#3=IFCWALL('0000000000000000000003',$,$,$,$,$,#30,$);
#30=IFCPRODUCTDEFINITIONSHAPE($,$,(#31));
#31=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#32));
#32=IFCCARTESIANPOINT((3.,0.,0.));
#60=IFCMATERIAL('Glass');
#61=IFCSTYLEDITEM($,(#52),$);
#62=IFCSTYLEDREPRESENTATION(#92,'Style','Material',(#61));
#63=IFCMATERIALDEFINITIONREPRESENTATION($,$,(#62),#60);
#64=IFCRELASSOCIATESMATERIAL('0000000000000000000064',$,$,$,(#3),#60);
";

#[test]
fn ifc2x3_styles_resolve_through_their_assignment_wrapper() {
    let read = |local| values_of(resolve_in("IFC2X3", IFC2X3, local).unwrap());
    assert_eq!(read("#1").unwrap(), [0.4]);
    assert_eq!(read("#2").unwrap(), [0.0]);
    // The material fallback waits on ifc-material binding to IFC2X3.
    assert!(matches!(
        resolve_in("IFC2X3", IFC2X3, "#3"),
        Err(PropertyResolutionError::Unavailable(message)) if message.contains("IFC2X3")
    ));
    // Without any styled material, an unstyled item needs no material.
    let unmaterialed: String = IFC2X3
        .lines()
        .filter(|line| !line.starts_with("#63="))
        .flat_map(|line| [line, "\n"])
        .collect();
    assert_eq!(
        values_of(resolve_in("IFC2X3", &unmaterialed, "#3").unwrap()),
        None
    );
}
