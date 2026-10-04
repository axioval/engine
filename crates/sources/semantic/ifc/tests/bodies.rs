//! How a body is modelled, read into the reserved body set: kind, profile,
//! placement and extrusion, in SI units and world coordinates.
#![allow(missing_docs)]

use std::f64::consts::FRAC_PI_2;

use axioval_engine::{
    PropertyRequest, PropertyResolution, PropertyResolutionError, PropertyResolutionServiceHandle,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{BODY_SET, ObjectId, PropertyValue, QuantityDimension, SourceId};

/// Millimetres and radians. Beam #10 is an HEA300 I-section extruded 6 m
/// along world x; column #30 a 300 x 400 rectangle extruded 3 m up through
/// a mapped representation; wall #50 a 5 m x 0.2 m rectangle extruded 3 m
/// up, voided by opening #60, a 1 m x 1.2 m rectangle extruded through it
/// along -y; member #80 two extrusions; slab #85 an open profile swept as a
/// solid; proxy #95 has no shape.
const DATA: &str = "\
#90=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#93=IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.);
#91=IFCUNITASSIGNMENT((#90,#93));
#92=IFCPROJECT('000000000000000000000P',$,'P',$,$,$,$,(#5),#91);
#1=IFCCARTESIANPOINT((0.,0.,0.));
#2=IFCAXIS2PLACEMENT3D(#1,$,$);
#4=IFCDIRECTION((0.,0.,1.));
#5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);
#13=IFCCARTESIANPOINT((1000.,0.,3000.));
#12=IFCAXIS2PLACEMENT3D(#13,$,$);
#11=IFCLOCALPLACEMENT($,#12);
#14=IFCISHAPEPROFILEDEF(.AREA.,'HEA300',$,300.,290.,8.5,14.,27.,$,$);
#16=IFCDIRECTION((1.,0.,0.));
#17=IFCDIRECTION((0.,1.,0.));
#15=IFCAXIS2PLACEMENT3D(#1,#16,#17);
#18=IFCEXTRUDEDAREASOLID(#14,#15,#4,6000.);
#19=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#18));
#20=IFCPRODUCTDEFINITIONSHAPE($,$,(#19));
#10=IFCBEAM('0000000000000000000010',$,'B1',$,$,#11,#20,$,.BEAM.);
#34=IFCRECTANGLEPROFILEDEF(.AREA.,'C300x400',$,300.,400.);
#33=IFCEXTRUDEDAREASOLID(#34,#2,#4,3000.);
#32=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#33));
#31=IFCREPRESENTATIONMAP(#2,#32);
#36=IFCCARTESIANTRANSFORMATIONOPERATOR3D($,$,#1,$,$);
#35=IFCMAPPEDITEM(#31,#36);
#37=IFCSHAPEREPRESENTATION(#5,'Body','MappedRepresentation',(#35));
#38=IFCPRODUCTDEFINITIONSHAPE($,$,(#37));
#40=IFCCARTESIANPOINT((5000.,0.,0.));
#41=IFCAXIS2PLACEMENT3D(#40,$,$);
#39=IFCLOCALPLACEMENT($,#41);
#30=IFCCOLUMN('0000000000000000000030',$,'C1',$,$,#39,#38,$,.COLUMN.);
#53=IFCCARTESIANPOINT((2500.,0.));
#52=IFCAXIS2PLACEMENT2D(#53,$);
#51=IFCRECTANGLEPROFILEDEF(.AREA.,$,#52,5000.,200.);
#54=IFCEXTRUDEDAREASOLID(#51,#2,#4,3000.);
#55=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#54));
#56=IFCPRODUCTDEFINITIONSHAPE($,$,(#55));
#57=IFCLOCALPLACEMENT($,#2);
#50=IFCWALL('0000000000000000000050',$,'W1',$,$,#57,#56,$,.STANDARD.);
#61=IFCLOCALPLACEMENT(#57,#2);
#62=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,1000.,1200.);
#64=IFCCARTESIANPOINT((1000.,100.,1500.));
#65=IFCDIRECTION((0.,-1.,0.));
#63=IFCAXIS2PLACEMENT3D(#64,#65,#16);
#67=IFCEXTRUDEDAREASOLID(#62,#63,#4,200.);
#68=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#67));
#69=IFCPRODUCTDEFINITIONSHAPE($,$,(#68));
#60=IFCOPENINGELEMENT('0000000000000000000060',$,'O1',$,$,#61,#69,$,.OPENING.);
#66=IFCRELVOIDSELEMENT('0000000000000000000066',$,$,$,#50,#60);
#81=IFCEXTRUDEDAREASOLID(#14,#15,#4,2000.);
#82=IFCEXTRUDEDAREASOLID(#34,#2,#4,20.);
#83=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#81,#82));
#84=IFCPRODUCTDEFINITIONSHAPE($,$,(#83));
#80=IFCMEMBER('0000000000000000000080',$,'M1',$,$,#57,#84,$,.MEMBER.);
#86=IFCCARTESIANPOINT((0.,0.));
#87=IFCCARTESIANPOINT((100.,0.));
#88=IFCPOLYLINE((#86,#87));
#89=IFCARBITRARYOPENPROFILEDEF(.CURVE.,$,#88);
#94=IFCEXTRUDEDAREASOLID(#89,#2,#4,100.);
#96=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#94));
#97=IFCPRODUCTDEFINITIONSHAPE($,$,(#96));
#85=IFCSLAB('0000000000000000000085',$,'S1',$,$,#57,#97,$,.FLOOR.);
#95=IFCBUILDINGELEMENTPROXY('0000000000000000000095',$,'X1',$,$,$,$,$,$);
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
    let request = PropertyRequest::try_new(object, Some(BODY_SET.into()), name).unwrap();
    session
        .service::<PropertyResolutionServiceHandle>()
        .unwrap()
        .resolve(&request)
}

fn value_in(schema: &str, data: &str, local: &str, name: &str) -> Option<PropertyValue> {
    match resolve_in(schema, data, local, name).unwrap() {
        PropertyResolution::Present(resolved) => Some(resolved.property().value.clone()),
        PropertyResolution::Absent(_) => None,
    }
}

fn value(local: &str, name: &str) -> Option<PropertyValue> {
    value_in("IFC4", DATA, local, name)
}

fn text(local: &str, name: &str) -> String {
    match value(local, name) {
        Some(PropertyValue::String(text)) => text,
        other => panic!("{name} is not text: {other:?}"),
    }
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

fn decimal(local: &str, name: &str) -> f64 {
    match value(local, name) {
        Some(PropertyValue::Decimal(value)) => value,
        other => panic!("{name} is not a decimal: {other:?}"),
    }
}

fn radians(local: &str, name: &str) -> f64 {
    match value(local, name) {
        Some(PropertyValue::Quantity {
            value,
            dimension: QuantityDimension::PlaneAngle,
        }) => value,
        other => panic!("{name} is not an angle: {other:?}"),
    }
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "{actual} is not {expected}"
    );
}

fn locator(local: &str, name: &str) -> String {
    let PropertyResolution::Present(resolved) = resolve_in("IFC4", DATA, local, name).unwrap()
    else {
        panic!("{name} is absent");
    };
    let evidence = resolved.property().evidence.clone().unwrap();
    assert!(evidence.exact, "{name} of {local} is not exact");
    // `ifc:<fingerprint>:<detail>`, the fingerprint itself `sha256:<hex>`.
    let start = evidence.locator.find(":body:").unwrap() + 1;
    evidence.locator[start..].to_owned()
}

#[test]
fn an_i_beam_states_its_section_and_extrusion_in_si() {
    assert_eq!(value("#10", "Count"), Some(PropertyValue::Integer(1)));
    assert_eq!(
        value("#10", "Kinds"),
        Some(PropertyValue::List(vec![PropertyValue::String(
            "extrusion".into()
        )]))
    );
    assert_eq!(text("#10", "Kind"), "extrusion");
    assert_eq!(text("#10", "item1.kind"), "extrusion");
    assert_eq!(value("#10", "Mapped"), Some(PropertyValue::Boolean(false)));
    assert_eq!(
        value("#10", "Mirrored"),
        Some(PropertyValue::Boolean(false))
    );
    assert_eq!(text("#10", "Profile.Type"), "i-shape");
    assert_eq!(text("#10", "Profile.Name"), "HEA300");
    close(metres("#10", "Profile.OverallWidth"), 0.3);
    close(metres("#10", "Profile.OverallDepth"), 0.29);
    close(metres("#10", "Profile.WebThickness"), 0.0085);
    close(metres("#10", "Profile.FlangeThickness"), 0.014);
    close(metres("#10", "Profile.FilletRadius"), 0.027);
    close(metres("#10", "Item1.Profile.OverallDepth"), 0.29);
    // Unset optional parameters and other families' parameters are absent.
    assert_eq!(value("#10", "Profile.FlangeSlope"), None);
    assert_eq!(value("#10", "Profile.XDim"), None);
    assert_eq!(value("#10", "Profile.PositionX"), None);
    // The beam runs along world x from its placement at (1, 0, 3).
    close(metres("#10", "Extrusion.Depth"), 6.0);
    close(decimal("#10", "Extrusion.DirectionX"), 1.0);
    close(decimal("#10", "Extrusion.DirectionZ"), 0.0);
    close(radians("#10", "Extrusion.Inclination"), FRAC_PI_2);
    close(metres("#10", "Placement.OriginX"), 1.0);
    close(metres("#10", "Placement.OriginZ"), 3.0);
    // The profile's Y axis points up.
    close(decimal("#10", "Placement.YAxisZ"), 1.0);
    assert_eq!(value("#10", "Revolution.Angle"), None);
    assert_eq!(value("#10", "Item2.Kind"), None);
    assert_eq!(
        locator("#10", "Profile.OverallDepth"),
        "body:#10:#19:#18:profile:#14"
    );
    assert_eq!(locator("#10", "Count"), "body:#10:#19");
}

#[test]
fn a_mapped_column_reads_as_if_authored_in_place() {
    assert_eq!(value("#30", "Mapped"), Some(PropertyValue::Boolean(true)));
    // A mapped swept solid is placed rigidly: it cannot be mirrored.
    assert_eq!(
        value("#30", "Mirrored"),
        Some(PropertyValue::Boolean(false))
    );
    assert_eq!(text("#30", "Profile.Type"), "rectangle");
    assert_eq!(text("#30", "Profile.Name"), "C300x400");
    close(metres("#30", "Profile.XDim"), 0.3);
    close(metres("#30", "Profile.YDim"), 0.4);
    close(metres("#30", "Extrusion.Depth"), 3.0);
    close(decimal("#30", "Extrusion.DirectionZ"), 1.0);
    close(radians("#30", "Extrusion.Inclination"), 0.0);
    close(metres("#30", "Placement.OriginX"), 5.0);
    assert_eq!(locator("#30", "Kind"), "body:#30:#37:#35>#33");
}

#[test]
fn a_wall_and_its_opening_state_their_profiles_and_directions() {
    assert_eq!(text("#50", "Profile.Type"), "rectangle");
    close(metres("#50", "Profile.XDim"), 5.0);
    close(metres("#50", "Profile.PositionX"), 2.5);
    close(radians("#50", "Profile.PositionAngle"), 0.0);
    close(radians("#50", "Extrusion.Inclination"), 0.0);
    // The opening is placed relative to the wall and extruded along -y.
    assert_eq!(text("#60", "Profile.Type"), "rectangle");
    close(metres("#60", "Profile.XDim"), 1.0);
    close(metres("#60", "Profile.YDim"), 1.2);
    close(decimal("#60", "Extrusion.DirectionY"), -1.0);
    close(radians("#60", "Extrusion.Inclination"), FRAC_PI_2);
    close(metres("#60", "Placement.OriginX"), 1.0);
    close(metres("#60", "Placement.OriginY"), 0.1);
    close(metres("#60", "Placement.OriginZ"), 1.5);
}

#[test]
fn a_body_of_several_items_is_read_item_by_item() {
    assert_eq!(value("#80", "Count"), Some(PropertyValue::Integer(2)));
    assert_eq!(
        value("#80", "Kinds"),
        Some(PropertyValue::List(vec![PropertyValue::String(
            "extrusion".into()
        )]))
    );
    assert_eq!(text("#80", "Item1.Profile.Type"), "i-shape");
    assert_eq!(text("#80", "Item2.Profile.Type"), "rectangle");
    close(metres("#80", "Item2.Extrusion.Depth"), 0.02);
    // No one item is "the" profile.
    assert!(matches!(
        resolve_in("IFC4", DATA, "#80", "Profile.Type"),
        Err(PropertyResolutionError::Conflicting(_))
    ));
    assert!(matches!(
        resolve_in("IFC4", DATA, "#80", "Kind"),
        Err(PropertyResolutionError::Conflicting(_))
    ));
    // A name no item has is absent, not ambiguous.
    assert_eq!(value("#80", "Revolution.Angle"), None);
}

#[test]
fn an_unreadable_body_is_refused_and_a_missing_one_is_absent() {
    // An open profile bounds no area: the body cannot be described.
    for name in ["Count", "Kind", "Profile.Type"] {
        assert!(
            matches!(
                resolve_in("IFC4", DATA, "#85", name),
                Err(PropertyResolutionError::Unavailable(_))
            ),
            "{name}"
        );
    }
    assert_eq!(value("#95", "Count"), None);
    assert_eq!(value("#95", "Kind"), None);
    // The project is no product.
    assert_eq!(value("#92", "Count"), None);
}

#[test]
fn an_unresolved_angle_unit_refuses_the_angles_alone() {
    let data = "\
#90=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#91=IFCUNITASSIGNMENT((#90));
#92=IFCPROJECT('000000000000000000000P',$,'P',$,$,$,$,(#5),#91);
#1=IFCCARTESIANPOINT((0.,0.,0.));
#2=IFCAXIS2PLACEMENT3D(#1,$,$);
#4=IFCDIRECTION((0.,0.,1.));
#5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);
#11=IFCLOCALPLACEMENT($,#2);
#14=IFCISHAPEPROFILEDEF(.AREA.,$,$,300.,290.,8.5,14.,$,$,0.1);
#18=IFCEXTRUDEDAREASOLID(#14,#2,#4,6000.);
#19=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#18));
#20=IFCPRODUCTDEFINITIONSHAPE($,$,(#19));
#10=IFCCOLUMN('0000000000000000000010',$,'C1',$,$,#11,#20,$,.COLUMN.);
";
    assert!(matches!(
        value_in("IFC4", data, "#10", "Profile.OverallDepth"),
        Some(PropertyValue::Quantity { value, .. }) if (value - 0.29).abs() < 1e-12
    ));
    assert!(matches!(
        resolve_in("IFC4", data, "#10", "Profile.FlangeSlope"),
        Err(PropertyResolutionError::Incomplete(_))
    ));
}

#[test]
fn an_ifc2x3_beam_is_read_in_its_own_release() {
    let data = "\
#90=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#93=IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.);
#91=IFCUNITASSIGNMENT((#90,#93));
#92=IFCPROJECT('000000000000000000000P',$,'P',$,$,$,$,(#5),#91);
#1=IFCCARTESIANPOINT((0.,0.,0.));
#2=IFCAXIS2PLACEMENT3D(#1,$,$);
#4=IFCDIRECTION((0.,0.,1.));
#5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);
#11=IFCLOCALPLACEMENT($,#2);
#14=IFCISHAPEPROFILEDEF(.AREA.,'IPE200',$,100.,200.,5.6,8.5,12.);
#18=IFCEXTRUDEDAREASOLID(#14,#2,#4,4000.);
#19=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#18));
#20=IFCPRODUCTDEFINITIONSHAPE($,$,(#19));
#10=IFCBEAM('0000000000000000000010',$,'B1',$,$,#11,#20,$);
";
    assert_eq!(
        value_in("IFC2X3", data, "#10", "Profile.Name"),
        Some(PropertyValue::String("IPE200".into()))
    );
    assert!(matches!(
        value_in("IFC2X3", data, "#10", "Profile.OverallDepth"),
        Some(PropertyValue::Quantity { value, .. }) if (value - 0.2).abs() < 1e-12
    ));
}

/// Millimetres. Wall #10 is extruded up from a mitred plan outline, a
/// polyline; slab #30 from an indexed outline with a square void; slab #50
/// from an outline with an arc, which no vertex list states.
const OUTLINES: &str = "\
#90=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#91=IFCUNITASSIGNMENT((#90));
#92=IFCPROJECT('000000000000000000000P',$,'P',$,$,$,$,(#5),#91);
#1=IFCCARTESIANPOINT((0.,0.,0.));
#2=IFCAXIS2PLACEMENT3D(#1,$,$);
#4=IFCDIRECTION((0.,0.,1.));
#5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);
#6=IFCLOCALPLACEMENT($,#2);
#11=IFCCARTESIANPOINT((0.,0.));
#12=IFCCARTESIANPOINT((5000.,0.));
#13=IFCCARTESIANPOINT((5200.,200.));
#14=IFCCARTESIANPOINT((0.,200.));
#15=IFCPOLYLINE((#11,#12,#13,#14,#11));
#16=IFCARBITRARYCLOSEDPROFILEDEF(.AREA.,'mitre',#15);
#17=IFCEXTRUDEDAREASOLID(#16,#2,#4,3000.);
#18=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#17));
#19=IFCPRODUCTDEFINITIONSHAPE($,$,(#18));
#10=IFCWALL('0000000000000000000010',$,'W1',$,$,#6,#19,$,.STANDARD.);
#31=IFCCARTESIANPOINTLIST2D(((0.,0.),(4000.,0.),(4000.,3000.),(0.,3000.)));
#32=IFCINDEXEDPOLYCURVE(#31,(IFCLINEINDEX((1,2,3,4,1))),$);
#33=IFCCARTESIANPOINTLIST2D(((1000.,1000.),(2000.,1000.),(2000.,2000.),(1000.,2000.)));
#34=IFCINDEXEDPOLYCURVE(#33,$,$);
#35=IFCARBITRARYPROFILEDEFWITHVOIDS(.AREA.,$,#32,(#34));
#36=IFCEXTRUDEDAREASOLID(#35,#2,#4,200.);
#37=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#36));
#38=IFCPRODUCTDEFINITIONSHAPE($,$,(#37));
#30=IFCSLAB('0000000000000000000030',$,'S1',$,$,#6,#38,$,.FLOOR.);
#51=IFCCARTESIANPOINTLIST2D(((0.,0.),(1000.,0.),(1500.,500.),(1000.,1000.),(0.,1000.)));
#52=IFCINDEXEDPOLYCURVE(#51,(IFCLINEINDEX((1,2)),IFCARCINDEX((2,3,4)),IFCLINEINDEX((4,5,1))),$);
#53=IFCARBITRARYCLOSEDPROFILEDEF(.AREA.,$,#52);
#54=IFCEXTRUDEDAREASOLID(#53,#2,#4,200.);
#55=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#54));
#56=IFCPRODUCTDEFINITIONSHAPE($,$,(#55));
#50=IFCSLAB('0000000000000000000050',$,'S2',$,$,#6,#56,$,.FLOOR.);
";

fn lengths(values: &[f64]) -> PropertyValue {
    PropertyValue::List(
        values
            .iter()
            .map(|value| PropertyValue::Quantity {
                value: *value,
                dimension: QuantityDimension::Length,
            })
            .collect(),
    )
}

#[test]
fn an_arbitrary_outline_states_its_vertices_and_voids() {
    let outline = |local: &str, name: &str| value_in("IFC4", OUTLINES, local, name);
    assert_eq!(
        outline("#10", "Profile.Type"),
        Some(PropertyValue::String("arbitrary-closed".into()))
    );
    // The polyline's closing vertex is not repeated.
    assert_eq!(
        outline("#10", "Profile.OutlineX"),
        Some(lengths(&[0.0, 5.0, 5.2, 0.0]))
    );
    assert_eq!(
        outline("#10", "Item1.Profile.OutlineY"),
        Some(lengths(&[0.0, 0.0, 0.2, 0.2]))
    );
    assert_eq!(outline("#10", "Profile.Void1.OutlineX"), None);
    assert_eq!(
        outline("#30", "Profile.OutlineX"),
        Some(lengths(&[0.0, 4.0, 4.0, 0.0]))
    );
    assert_eq!(
        outline("#30", "Profile.VoidCount"),
        Some(PropertyValue::Integer(1))
    );
    assert_eq!(
        outline("#30", "Profile.Void1.OutlineX"),
        Some(lengths(&[1.0, 2.0, 2.0, 1.0]))
    );
    assert_eq!(
        outline("#30", "Profile.Void1.OutlineY"),
        Some(lengths(&[1.0, 1.0, 2.0, 2.0]))
    );
    assert_eq!(outline("#30", "Profile.Void2.OutlineX"), None);
    // Parameterised families state no outline.
    assert_eq!(value("#50", "Profile.OutlineX"), None);
}

#[test]
fn a_curved_outline_is_refused_and_the_rest_of_the_profile_stands() {
    for name in ["Profile.OutlineX", "Profile.OutlineY"] {
        assert!(
            matches!(
                resolve_in("IFC4", OUTLINES, "#50", name),
                Err(PropertyResolutionError::Unavailable(message)) if message.contains("vertices")
            ),
            "{name}"
        );
    }
    assert_eq!(
        value_in("IFC4", OUTLINES, "#50", "Profile.Type"),
        Some(PropertyValue::String("arbitrary-closed".into()))
    );
    assert!(matches!(
        value_in("IFC4", OUTLINES, "#50", "Extrusion.Depth"),
        Some(PropertyValue::Quantity { value, .. }) if (value - 0.2).abs() < 1e-12
    ));
}

/// A tetrahedron as a faceted B-rep, mapped into proxies #110, #120 and #130.
/// IFC has no negative scale: #110's operator mirrors by an `Axis2` opposing
/// `Axis3 × Axis1`; #120's operator is the identity; #130 places the B-rep
/// once mirrored and once not.
const MAPPED_BREPS: &str = "\
#90=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#91=IFCUNITASSIGNMENT((#90));
#92=IFCPROJECT('000000000000000000000P',$,'P',$,$,$,$,(#5),#91);
#1=IFCCARTESIANPOINT((0.,0.,0.));
#2=IFCAXIS2PLACEMENT3D(#1,$,$);
#5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);
#61=IFCCARTESIANPOINT((1000.,0.,0.));
#62=IFCCARTESIANPOINT((0.,1000.,0.));
#63=IFCCARTESIANPOINT((0.,0.,1000.));
#64=IFCPOLYLOOP((#1,#62,#61));
#65=IFCPOLYLOOP((#1,#61,#63));
#66=IFCPOLYLOOP((#1,#63,#62));
#67=IFCPOLYLOOP((#61,#62,#63));
#68=IFCFACEOUTERBOUND(#64,.T.);
#69=IFCFACEOUTERBOUND(#65,.T.);
#70=IFCFACEOUTERBOUND(#66,.T.);
#71=IFCFACEOUTERBOUND(#67,.T.);
#72=IFCFACE((#68));
#73=IFCFACE((#69));
#74=IFCFACE((#70));
#75=IFCFACE((#71));
#76=IFCCLOSEDSHELL((#72,#73,#74,#75));
#77=IFCFACETEDBREP(#76);
#78=IFCSHAPEREPRESENTATION(#5,'Body','Brep',(#77));
#79=IFCREPRESENTATIONMAP(#2,#78);
#81=IFCDIRECTION((1.,0.,0.));
#82=IFCDIRECTION((0.,-1.,0.));
#83=IFCDIRECTION((0.,0.,1.));
#84=IFCCARTESIANTRANSFORMATIONOPERATOR3D(#81,#82,#1,$,#83);
#85=IFCCARTESIANTRANSFORMATIONOPERATOR3D($,$,#1,$,$);
#86=IFCMAPPEDITEM(#79,#84);
#87=IFCMAPPEDITEM(#79,#85);
#88=IFCLOCALPLACEMENT($,#2);
#111=IFCSHAPEREPRESENTATION(#5,'Body','MappedRepresentation',(#86));
#112=IFCPRODUCTDEFINITIONSHAPE($,$,(#111));
#110=IFCBUILDINGELEMENTPROXY('0000000000000000000110',$,'M',$,$,#88,#112,$,$);
#121=IFCSHAPEREPRESENTATION(#5,'Body','MappedRepresentation',(#87));
#122=IFCPRODUCTDEFINITIONSHAPE($,$,(#121));
#120=IFCBUILDINGELEMENTPROXY('0000000000000000000120',$,'N',$,$,#88,#122,$,$);
#131=IFCSHAPEREPRESENTATION(#5,'Body','MappedRepresentation',(#86,#87));
#132=IFCPRODUCTDEFINITIONSHAPE($,$,(#131));
#130=IFCBUILDINGELEMENTPROXY('0000000000000000000130',$,'B',$,$,#88,#132,$,$);
";

#[test]
fn a_mapping_states_whether_it_mirrors_any_item_kind() {
    let mirrored = |local| value_in("IFC4", MAPPED_BREPS, local, "Mirrored");
    assert_eq!(
        value_in("IFC4", MAPPED_BREPS, "#110", "Mapped"),
        Some(PropertyValue::Boolean(true))
    );
    assert_eq!(mirrored("#110"), Some(PropertyValue::Boolean(true)));
    assert_eq!(mirrored("#120"), Some(PropertyValue::Boolean(false)));
    // One body mirrored in part is neither.
    assert!(matches!(
        resolve_in("IFC4", MAPPED_BREPS, "#130", "Mirrored"),
        Err(PropertyResolutionError::Conflicting(_))
    ));
}

/// Site #200 is a 100 m x 50 m plot 1 m thick; site #210 states no shape;
/// site #220 states a footprint alone; site #230 an open profile swept as a
/// solid, which cannot be described.
const SITES: &str = "\
#90=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);
#91=IFCUNITASSIGNMENT((#90));
#92=IFCPROJECT('000000000000000000000P',$,'P',$,$,$,$,(#5),#91);
#1=IFCCARTESIANPOINT((0.,0.,0.));
#2=IFCAXIS2PLACEMENT3D(#1,$,$);
#4=IFCDIRECTION((0.,0.,1.));
#5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);
#3=IFCLOCALPLACEMENT($,#2);
#201=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,100.,50.);
#202=IFCEXTRUDEDAREASOLID(#201,#2,#4,1.);
#203=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#202));
#204=IFCPRODUCTDEFINITIONSHAPE($,$,(#203));
#200=IFCSITE('0000000000000000000200',$,'S1',$,$,#3,#204,$,.ELEMENT.,$,$,$,$,$);
#210=IFCSITE('0000000000000000000210',$,'S2',$,$,#3,$,$,.ELEMENT.,$,$,$,$,$);
#221=IFCCARTESIANPOINT((100.,0.));
#222=IFCCARTESIANPOINT((100.,50.));
#223=IFCCARTESIANPOINT((0.,50.));
#224=IFCCARTESIANPOINT((0.,0.));
#225=IFCPOLYLINE((#224,#221,#222,#223,#224));
#226=IFCSHAPEREPRESENTATION(#5,'FootPrint','Curve2D',(#225));
#227=IFCPRODUCTDEFINITIONSHAPE($,$,(#226));
#220=IFCSITE('0000000000000000000220',$,'S3',$,$,#3,#227,$,.ELEMENT.,$,$,$,$,$);
#231=IFCPOLYLINE((#224,#221));
#232=IFCARBITRARYOPENPROFILEDEF(.CURVE.,$,#231);
#233=IFCEXTRUDEDAREASOLID(#232,#2,#4,1.);
#234=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#233));
#235=IFCPRODUCTDEFINITIONSHAPE($,$,(#234));
#230=IFCSITE('0000000000000000000230',$,'S4',$,$,#3,#235,$,.ELEMENT.,$,$,$,$,$);
";

/// Whether a site has geometry is whether it states `axioval:body.Count`:
/// its absence is read from the representations, never from a missing
/// service.
#[test]
fn a_site_has_a_body_only_where_its_representations_state_one() {
    let site = |local| value_in("IFC4", SITES, local, "Count");
    assert_eq!(site("#200"), Some(PropertyValue::Integer(1)));
    // No shape, and a footprint that is no body, are exact absences.
    assert_eq!(site("#210"), None);
    assert_eq!(site("#220"), None);
    // A body the source states but that cannot be read is refused.
    assert!(matches!(
        resolve_in("IFC4", SITES, "#230", "Count"),
        Err(PropertyResolutionError::Unavailable(_))
    ));
}

/// An IFC4X3 deck lofted between stations along its directrix
/// (`IfcSectionedSolidHorizontal`) is a sectioned spine since
/// `ifc-geometry` 0.9 (openbimrs/ifc#307).
#[test]
fn an_ifc4x3_sectioned_solid_is_a_sectioned_spine() {
    let data = "\
#90=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);
#91=IFCUNITASSIGNMENT((#90));
#92=IFCPROJECT('000000000000000000000P',$,'P',$,$,$,$,(#5),#91);
#1=IFCCARTESIANPOINT((0.,0.,0.));
#2=IFCAXIS2PLACEMENT3D(#1,$,$);
#3=IFCLOCALPLACEMENT($,#2);
#5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);
#10=IFCPOLYLINE((#11,#12));
#11=IFCCARTESIANPOINT((0.,0.,0.));
#12=IFCCARTESIANPOINT((10.,0.,0.));
#13=IFCRECTANGLEPROFILEDEF(.AREA.,'narrow',#15,2.,1.);
#14=IFCRECTANGLEPROFILEDEF(.AREA.,'wide',#15,4.,1.);
#15=IFCAXIS2PLACEMENT2D(#16,$);
#16=IFCCARTESIANPOINT((0.,0.));
#17=IFCAXIS2PLACEMENTLINEAR(#18,$,$);
#18=IFCPOINTBYDISTANCEEXPRESSION(IFCLENGTHMEASURE(0.),$,$,$,#10);
#19=IFCAXIS2PLACEMENTLINEAR(#20,$,$);
#20=IFCPOINTBYDISTANCEEXPRESSION(IFCLENGTHMEASURE(10.),$,$,$,#10);
#21=IFCSECTIONEDSOLIDHORIZONTAL(#10,(#13,#14),(#17,#19));
#22=IFCSHAPEREPRESENTATION(#5,'Body','AdvancedSweptSolid',(#21));
#23=IFCPRODUCTDEFINITIONSHAPE($,$,(#22));
#30=IFCBUILDINGELEMENTPROXY('0000000000000000000030',$,'Deck',$,$,#3,#23,$,$);
";
    assert_eq!(
        value_in("IFC4X3_ADD2", data, "#30", "Count"),
        Some(PropertyValue::Integer(1))
    );
    assert_eq!(
        value_in("IFC4X3_ADD2", data, "#30", "Kind"),
        Some(PropertyValue::String("sectioned-spine".into()))
    );
    // Its sections are no swept-area profile.
    assert_eq!(value_in("IFC4X3_ADD2", data, "#30", "Profile.Type"), None);
}
