//! Whether a body item is glazing, from its own styles, a styled mapped
//! item above it, or its product's material styles (#205).
#![allow(missing_docs)]

use axioval_ifc::{GlazingReader, ItemGlazing, read_ifc_step};
use ifc_model::{EntityId, Model};

/// Styles: #50 glass (0.7), #51 opaque, #52 matte material (0).
///
/// Door #1: #12 styled glass, #13 opaque, #14 unstyled, #15 styled glass
/// and opaque by its layers, #16 a mapped item styled glass whose map
/// holds the unstyled #18. Door #2 (material #200, glass) has the
/// unstyled #22; door #3 (materials #200 and #210, glass and matte) the
/// unstyled #32; door #4 (material #220, no style) the unstyled #42.
const DATA: &str = "\
#90=IFCCARTESIANPOINT((0.,0.,0.));
#91=IFCAXIS2PLACEMENT3D(#90,$,$);
#92=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#91,$);
#40=IFCCOLOURRGB($,0.5,0.5,0.5);
#41=IFCSURFACESTYLESHADING(#40,0.7);
#42=IFCSURFACESTYLESHADING(#40,0.);
#50=IFCSURFACESTYLE('Glass',.BOTH.,(#41));
#51=IFCSURFACESTYLE('Frame',.BOTH.,(#42));
#52=IFCSURFACESTYLE('Matte',.BOTH.,(#42));
#1=IFCDOOR('0000000000000000000001',$,'D1',$,$,$,#10,$,$,$,$,$,$);
#10=IFCPRODUCTDEFINITIONSHAPE($,$,(#11));
#11=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#12,#13,#14,#15,#16));
#12=IFCCARTESIANPOINT((1.,0.,0.));
#13=IFCCARTESIANPOINT((2.,0.,0.));
#14=IFCCARTESIANPOINT((3.,0.,0.));
#15=IFCCARTESIANPOINT((4.,0.,0.));
#16=IFCMAPPEDITEM(#17,#19);
#17=IFCREPRESENTATIONMAP(#91,#25);
#25=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#18));
#18=IFCCARTESIANPOINT((5.,0.,0.));
#19=IFCCARTESIANTRANSFORMATIONOPERATOR3D($,$,#90,$,$);
#60=IFCSTYLEDITEM(#12,(#50),$);
#61=IFCSTYLEDITEM(#13,(#51),$);
#62=IFCSTYLEDITEM(#16,(#50),$);
#63=IFCPRESENTATIONLAYERWITHSTYLE('Glazing',$,(#15),$,.T.,.F.,.F.,(#50));
#64=IFCPRESENTATIONLAYERWITHSTYLE('Frames',$,(#15),$,.T.,.F.,.F.,(#51));
#2=IFCDOOR('0000000000000000000002',$,'D2',$,$,$,#20,$,$,$,$,$,$);
#20=IFCPRODUCTDEFINITIONSHAPE($,$,(#21));
#21=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#22));
#22=IFCCARTESIANPOINT((6.,0.,0.));
#3=IFCDOOR('0000000000000000000003',$,'D3',$,$,$,#30,$,$,$,$,$,$);
#30=IFCPRODUCTDEFINITIONSHAPE($,$,(#31));
#31=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#32));
#32=IFCCARTESIANPOINT((7.,0.,0.));
#4=IFCDOOR('0000000000000000000004',$,'D4',$,$,$,#43,$,$,$,$,$,$);
#43=IFCPRODUCTDEFINITIONSHAPE($,$,(#44));
#44=IFCSHAPEREPRESENTATION(#92,'Body','Brep',(#45));
#45=IFCCARTESIANPOINT((8.,0.,0.));
#200=IFCMATERIAL('Glass',$,$);
#201=IFCSTYLEDITEM($,(#50),$);
#202=IFCSTYLEDREPRESENTATION(#92,'Style','Material',(#201));
#203=IFCMATERIALDEFINITIONREPRESENTATION($,$,(#202),#200);
#204=IFCRELASSOCIATESMATERIAL('0000000000000000000204',$,$,$,(#2),#200);
#210=IFCMATERIAL('Oak',$,$);
#211=IFCSTYLEDITEM($,(#52),$);
#212=IFCSTYLEDREPRESENTATION(#92,'Style','Material',(#211));
#213=IFCMATERIALDEFINITIONREPRESENTATION($,$,(#212),#210);
#214=IFCMATERIALLIST((#200,#210));
#215=IFCRELASSOCIATESMATERIAL('0000000000000000000215',$,$,$,(#3),#214);
#220=IFCMATERIAL('Paint',$,$);
#221=IFCRELASSOCIATESMATERIAL('0000000000000000000221',$,$,$,(#4),#220);
";

fn model() -> Model {
    let step = format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n{DATA}ENDSEC;\nEND-ISO-10303-21;\n"
    );
    read_ifc_step(step.as_bytes()).unwrap()
}

fn glazing(product: u64, mapped_by: &[u64], item: u64) -> ItemGlazing {
    let model = model();
    let reader = GlazingReader::new(&model).unwrap();
    let mapped: Vec<EntityId> = mapped_by.iter().map(|id| EntityId(*id)).collect();
    reader.item_glazing(&model, EntityId(product), &mapped, EntityId(item))
}

fn undecided(glazing: &ItemGlazing) -> &str {
    match glazing {
        ItemGlazing::Undecided(why) => why,
        other => panic!("decided: {other:?}"),
    }
}

#[test]
fn an_items_own_style_decides_it() {
    assert_eq!(glazing(1, &[], 12), ItemGlazing::Glazed);
    assert_eq!(glazing(1, &[], 13), ItemGlazing::Opaque);
}

#[test]
fn a_styled_mapped_item_styles_what_it_maps_in() {
    assert_eq!(glazing(1, &[16], 18), ItemGlazing::Glazed);
}

#[test]
fn styles_stating_glass_and_frame_decide_nothing() {
    assert_eq!(
        undecided(&glazing(1, &[], 15)),
        "its surface styles state it both transparent and opaque"
    );
}

#[test]
fn an_unstyled_item_without_a_material_is_undecided() {
    assert_eq!(
        undecided(&glazing(1, &[], 14)),
        "it has no surface style of its own and its material states none"
    );
}

#[test]
fn an_unstyled_item_takes_its_one_materials_style() {
    assert_eq!(glazing(2, &[], 22), ItemGlazing::Glazed);
}

#[test]
fn materials_that_disagree_leave_an_unstyled_item_undecided() {
    assert!(undecided(&glazing(3, &[], 32)).contains("transparent and opaque"));
    assert!(undecided(&glazing(4, &[], 45)).contains("#220 states none"));
}
