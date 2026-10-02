//! A source's coordinate system comes from its model context, in metres.
#![allow(missing_docs)]
// Stated values are read back unchanged, so exact comparison is the test.
#![allow(clippy::float_cmp)]

use axioval_engine::{
    CoordinateSystemError, CoordinateSystemServiceHandle, SitePlacement, SourceCoordinateSystem,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::SourceId;

/// Millimetres. The model context's world frame sits 1 m east and 2 m north
/// of the origin, turned a quarter to the left; true north is north-east.
const CONTEXT: &str = "\
#1=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#2=IFCUNITASSIGNMENT((#1));
#3=IFCPROJECT('0000000000000000000003',$,'P',$,$,$,$,(#5),#2);
#10=IFCCARTESIANPOINT((1000.,2000.,0.));
#11=IFCDIRECTION((0.,0.,1.));
#12=IFCDIRECTION((0.,1.,0.));
#13=IFCAXIS2PLACEMENT3D(#10,#11,#12);
#14=IFCDIRECTION((1.,1.));
#5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#13,#14);
#6=IFCGEOMETRICREPRESENTATIONSUBCONTEXT('Body','Model',*,*,*,*,#5,$,.MODEL_VIEW.,$);
";

/// A map conversion of the model context onto a metre-based projection,
/// rotated a quarter, with no scale stated.
const MAP_IN_METRES: &str = "\
#20=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);
#21=IFCPROJECTEDCRS('EPSG:25832',$,$,$,$,$,#20);
#22=IFCMAPCONVERSION(#5,#21,500000.,5600000.,50.,0.,1.,$);
";

/// The file with one wall, so the session has an object.
fn file(schema: &str, data: &str) -> String {
    let data = format!("{data}#99=IFCWALL('0000000000000000000099',$,$,$,$,$,$,$,$);\n");
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('{schema}'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

fn system(schema: &str, data: &str) -> Result<SourceCoordinateSystem, CoordinateSystemError> {
    let session = import_ifc_session("model.ifc", file(schema, data).as_bytes()).unwrap();
    session
        .service::<CoordinateSystemServiceHandle>()
        .expect("the IFC session registers its coordinate system without geometry")
        .coordinate_system(&SourceId::new("ifc-step", "model.ifc").unwrap())
}

fn close(actual: &[f64], expected: &[f64]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, e)| (a - e).abs() <= 1e-12)
}

#[test]
fn the_model_context_states_world_frame_true_north_and_map_conversion() {
    let system = system("IFC4", &format!("{CONTEXT}{MAP_IN_METRES}")).unwrap();
    let world = system.world().expect("the model context has a world frame");
    assert!(close(&world.origin_metres(), &[1.0, 2.0, 0.0]));
    let [x, y, z] = world.axes().map(|axis| axis.components());
    assert!(close(&x, &[0.0, 1.0, 0.0]), "{x:?}");
    assert!(close(&y, &[-1.0, 0.0, 0.0]), "{y:?}");
    assert!(close(&z, &[0.0, 0.0, 1.0]), "{z:?}");
    let half = std::f64::consts::FRAC_1_SQRT_2;
    assert!(close(&system.true_north().unwrap(), &[half, half]));

    let map = system.map().expect("the model is georeferenced");
    assert_eq!(map.target(), Some("EPSG:25832"));
    assert_eq!(map.offset(), [500_000.0, 5_600_000.0, 50.0]);
    assert_eq!(map.x_axis(), [0.0, 1.0]);
    assert!(
        (map.scale() - 1.0).abs() < f64::EPSILON,
        "an unset scale is 1"
    );
    assert_eq!(map.metres_per_map_unit(), Some(1.0));
    assert!(system.evidence().exact);
    assert!(
        system
            .evidence()
            .locator
            .contains(":coordinate-system:#5:#22")
    );
}

#[test]
fn an_unstated_map_unit_is_unknown_not_metres() {
    let data = format!(
        "{CONTEXT}\
         #21=IFCPROJECTEDCRS('EPSG:25832',$,$,$,$,$,$);\n\
         #22=IFCMAPCONVERSION(#5,#21,500000.,5600000.,50.,$,$,0.9996);\n"
    );
    let system = system("IFC4", &data).unwrap();
    let map = system.map().unwrap();
    assert_eq!(map.metres_per_map_unit(), None);
    assert_eq!(map.offset_metres(), None);
    assert_eq!(map.x_axis(), [1.0, 0.0], "an unset rotation is none");
    assert!((map.scale() - 0.9996).abs() < f64::EPSILON);
}

#[test]
fn a_file_without_a_model_context_states_no_coordinate_system() {
    let data = "\
#1=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);
#2=IFCUNITASSIGNMENT((#1));
#3=IFCPROJECT('0000000000000000000003',$,'P',$,$,$,$,$,#2);
";
    let system = system("IFC4", data).unwrap();
    assert!(system.world().is_none());
    assert!(system.true_north().is_none());
    assert!(system.map().is_none());
}

#[test]
fn ifc2x3_states_no_map_conversion() {
    let system = system("IFC2X3", CONTEXT).unwrap();
    assert!(system.world().is_some());
    assert!(system.map().is_none());
}

#[test]
fn ambiguous_statements_are_refused() {
    let second_context =
        format!("{CONTEXT}#7=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#13,$);\n");
    assert!(matches!(
        system("IFC4", &second_context),
        Err(CoordinateSystemError::Ambiguous(_))
    ));
    let second_map =
        format!("{CONTEXT}{MAP_IN_METRES}#23=IFCMAPCONVERSION(#5,#21,0.,0.,0.,$,$,$);\n");
    assert!(matches!(
        system("IFC4", &second_map),
        Err(CoordinateSystemError::Ambiguous(_))
    ));
    let from_elsewhere = format!(
        "{CONTEXT}#20=IFCPROJECTEDCRS('Local',$,$,$,$,$,$);\n\
         #21=IFCPROJECTEDCRS('EPSG:25832',$,$,$,$,$,$);\n\
         #22=IFCMAPCONVERSION(#20,#21,0.,0.,0.,$,$,$);\n"
    );
    assert!(matches!(
        system("IFC4", &from_elsewhere),
        Err(CoordinateSystemError::Unsupported(_))
    ));
}

#[test]
fn a_world_frame_without_an_exact_length_unit_is_refused() {
    let data = CONTEXT.replace(
        "#1=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);\n#2=IFCUNITASSIGNMENT((#1));",
        "#2=IFCUNITASSIGNMENT(());",
    );
    assert!(matches!(
        system("IFC4", &data),
        Err(CoordinateSystemError::Unreadable(_))
    ));
}

/// A site placed 3 m east of the origin (in millimetres), turned a quarter.
const SITE: &str = "\
#30=IFCCARTESIANPOINT((3000.,0.,0.));
#31=IFCAXIS2PLACEMENT3D(#30,#11,#12);
#32=IFCLOCALPLACEMENT($,#31);
#33=IFCSITE('0000000000000000000033',$,'Site',$,$,#32,$,$,.ELEMENT.,$,$,$,$,$);
";

#[test]
fn the_one_site_states_its_placement_in_metres() {
    let system = system("IFC4", &format!("{CONTEXT}{SITE}")).unwrap();
    let SitePlacement::Stated(site) = system.site() else {
        panic!("one site is stated: {:?}", system.site());
    };
    assert!(close(&site.origin_metres(), &[3.0, 0.0, 0.0]));
    assert!(close(&site.axes()[0].components(), &[0.0, 1.0, 0.0]));
    assert!(
        system.evidence().locator.ends_with(":site:#33"),
        "{}",
        system.evidence().locator
    );
}

#[test]
fn no_site_is_absent_and_several_or_unplaced_sites_are_unknown() {
    assert_eq!(
        system("IFC4", CONTEXT).unwrap().site(),
        &SitePlacement::Absent
    );
    let second = format!(
        "{CONTEXT}{SITE}#34=IFCSITE('0000000000000000000034',$,'Other',$,$,#32,$,$,.ELEMENT.,$,$,$,$,$);\n"
    );
    assert!(matches!(
        system("IFC4", &second).unwrap().site(),
        SitePlacement::Unknown(reason) if reason.contains("2 sites")
    ));
    let unplaced = format!(
        "{CONTEXT}#33=IFCSITE('0000000000000000000033',$,'Site',$,$,$,$,$,.ELEMENT.,$,$,$,$,$);\n"
    );
    assert!(matches!(
        system("IFC4", &unplaced).unwrap().site(),
        SitePlacement::Unknown(reason) if reason.contains("ObjectPlacement is not set")
    ));
}
