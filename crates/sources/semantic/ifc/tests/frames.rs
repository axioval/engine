//! Object frames come from IFC object placements, in metres.
#![allow(missing_docs)]

use axioval_engine::{ObjectFrame, ObjectFrameError, ObjectFrameServiceHandle, ObjectFront};
use axioval_ifc::import_ifc_session;
use axioval_ir::{ObjectId, SourceId};

/// Millimetres. A site 1 m east of the origin; a storey 2 m north of it,
/// 0.5 m up, turned a quarter to the left; walls placed in the storey.
const DATA: &str = "\
#1=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#2=IFCUNITASSIGNMENT((#1));
#3=IFCPROJECT('0000000000000000000003',$,'P',$,$,$,$,$,#2);
#10=IFCCARTESIANPOINT((1000.,0.,0.));
#11=IFCAXIS2PLACEMENT3D(#10,$,$);
#12=IFCLOCALPLACEMENT($,#11);
#13=IFCSITE('000000000000000000000D',$,'S',$,$,#12,$,$,.ELEMENT.,$,$,$,$,$);
#20=IFCCARTESIANPOINT((0.,2000.,500.));
#21=IFCDIRECTION((0.,0.,1.));
#22=IFCDIRECTION((0.,1.,0.));
#23=IFCAXIS2PLACEMENT3D(#20,#21,#22);
#24=IFCLOCALPLACEMENT(#12,#23);
#25=IFCBUILDINGSTOREY('000000000000000000000P',$,'1',$,$,#24,$,$,.ELEMENT.,500.);
#30=IFCCARTESIANPOINT((1000.,0.,0.));
#31=IFCAXIS2PLACEMENT3D(#30,$,$);
#32=IFCLOCALPLACEMENT(#24,#31);
#33=IFCWALL('000000000000000000000X',$,'Nested',$,$,#32,$,$,$);
#40=IFCCARTESIANPOINT((0.,0.,0.));
#41=IFCDIRECTION((0.,0.,-1.));
#42=IFCDIRECTION((1.,0.,0.));
#43=IFCAXIS2PLACEMENT3D(#40,#41,#42);
#44=IFCLOCALPLACEMENT($,#43);
#45=IFCDOOR('000000000000000000000Y',$,'Flipped',$,$,#44,$,$,$,$,$,$,$);
#50=IFCDIRECTION((1.,0.,1.));
#51=IFCAXIS2PLACEMENT3D(#40,$,#50);
#52=IFCLOCALPLACEMENT($,#51);
#53=IFCWALL('000000000000000000000Z',$,'Skewed',$,$,#52,$,$,$);
#54=IFCWALL('000000000000000000000a',$,'Unplaced',$,$,$,$,$,$);
#55=IFCGROUP('000000000000000000000b',$,'G',$,$);
#60=IFCGRIDPLACEMENT($,$);
#61=IFCWALL('000000000000000000000c',$,'OnGrid',$,$,#60,$,$,$);
#62=IFCLOCALPLACEMENT(#60,#11);
#63=IFCWALL('000000000000000000000e',$,'BelowGrid',$,$,#62,$,$,$);
#70=IFCAXIS2PLACEMENT3D(#40,#21,#21);
#71=IFCLOCALPLACEMENT($,#70);
#72=IFCWALL('000000000000000000000f',$,'Degenerate',$,$,#71,$,$,$);
";

fn file(schema: &str, data: &str) -> String {
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('{schema}'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

fn frame_in(schema: &str, data: &str, local: &str) -> Result<ObjectFrame, ObjectFrameError> {
    let session = import_ifc_session("model.ifc", file(schema, data).as_bytes()).unwrap();
    let object = ObjectId::new(SourceId::new("ifc-step", "model.ifc").unwrap(), local).unwrap();
    session
        .service::<ObjectFrameServiceHandle>()
        .expect("the IFC session registers object frames without geometry")
        .object_frame(&object)
}

fn frame(local: &str) -> Result<ObjectFrame, ObjectFrameError> {
    frame_in("IFC4", DATA, local)
}

fn close(actual: [f64; 3], expected: [f64; 3]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, e)| (a - e).abs() <= 1e-12)
}

fn assert_frame(
    frame: &ObjectFrame,
    origin: [f64; 3],
    right: [f64; 3],
    forward: [f64; 3],
    up: [f64; 3],
) {
    let metric = frame.frame();
    let actual = [
        metric.origin().coordinates_metres(),
        metric.right().components(),
        metric.forward().components(),
        metric.up().components(),
    ];
    for (actual, expected) in actual.iter().zip([origin, right, forward, up]) {
        assert!(close(*actual, expected), "{actual:?} != {expected:?}");
    }
}

#[test]
fn a_nested_rotated_chain_composes_to_metres() {
    let wall = frame("#33").unwrap();
    // Storey origin (1, 2, 0.5) m with its X along world Y; the wall is
    // 1000 mm along the storey's X.
    assert_frame(
        &wall,
        [1.0, 3.0, 0.5],
        [0.0, 1.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
    );
    assert_eq!(wall.object().local_id, "#33");
    assert!(wall.evidence().exact);
    assert!(
        wall.evidence()
            .locator
            .ends_with(":placement:#33:#32<#24<#12"),
        "{}",
        wall.evidence().locator
    );
    let storey = frame("#25").unwrap();
    assert_frame(
        &storey,
        [1.0, 2.0, 0.5],
        [0.0, 1.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
    );
}

#[test]
fn ifc_states_no_front() {
    for object in ["#13", "#25", "#33", "#45"] {
        assert_eq!(frame(object).unwrap().front(), ObjectFront::NotStated);
    }
}

#[test]
fn a_flipped_placement_is_reported_as_the_rotation_it_is() {
    // Axis (0,0,-1) with RefDirection (1,0,0): the usual way to mirror a
    // door in plan. IfcAxis2Placement3D derives Y as Z x X, so the frame is
    // still right-handed, with up pointing down and forward to -Y.
    let door = frame("#45").unwrap();
    assert_frame(
        &door,
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, 0.0, -1.0],
    );
}

#[test]
fn default_and_skewed_axes_follow_the_schema() {
    // Default Axis; RefDirection (1,0,1) is projected onto the XY plane.
    let wall = frame("#53").unwrap();
    assert_frame(
        &wall,
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
    );
}

#[test]
fn metre_files_are_not_scaled() {
    let metres = DATA.replace(".MILLI.", "$");
    let wall = frame_in("IFC4", &metres, "#33").unwrap();
    assert_frame(
        &wall,
        [1000.0, 3000.0, 500.0],
        [0.0, 1.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
    );
}

#[test]
fn an_unresolvable_length_unit_is_refused() {
    let unitless = DATA.replace("#2=IFCUNITASSIGNMENT((#1));", "#2=IFCUNITASSIGNMENT(());");
    assert!(matches!(
        frame_in("IFC4", &unitless, "#33"),
        Err(ObjectFrameError::Unreadable(_))
    ));
}

#[test]
fn objects_without_a_placement_have_no_frame() {
    assert!(matches!(frame("#54"), Err(ObjectFrameError::NotPlaced(_))));
    assert!(matches!(frame("#55"), Err(ObjectFrameError::NotPlaced(_))));
    assert!(matches!(
        frame("#999"),
        Err(ObjectFrameError::UnknownObject(_))
    ));
}

#[test]
fn grid_placements_are_refused() {
    assert!(matches!(
        frame("#61"),
        Err(ObjectFrameError::Unsupported(_))
    ));
    assert!(matches!(
        frame("#63"),
        Err(ObjectFrameError::Unsupported(_))
    ));
}

#[test]
fn parallel_axes_are_refused() {
    assert!(matches!(frame("#72"), Err(ObjectFrameError::Unreadable(_))));
}

/// The nested chain in IFC2X3, whose wall has no `PredefinedType`.
const IFC2X3_DATA: &str = "\
#1=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#2=IFCUNITASSIGNMENT((#1));
#3=IFCPROJECT('0000000000000000000003',$,'P',$,$,$,$,$,#2);
#10=IFCCARTESIANPOINT((1000.,0.,0.));
#11=IFCAXIS2PLACEMENT3D(#10,$,$);
#12=IFCLOCALPLACEMENT($,#11);
#20=IFCCARTESIANPOINT((0.,2000.,500.));
#21=IFCDIRECTION((0.,0.,1.));
#22=IFCDIRECTION((0.,1.,0.));
#23=IFCAXIS2PLACEMENT3D(#20,#21,#22);
#24=IFCLOCALPLACEMENT(#12,#23);
#30=IFCCARTESIANPOINT((1000.,0.,0.));
#31=IFCAXIS2PLACEMENT3D(#30,$,$);
#32=IFCLOCALPLACEMENT(#24,#31);
#33=IFCWALL('000000000000000000000X',$,'Nested',$,$,#32,$,$);
";

#[test]
fn ifc2x3_reads_the_placement_slot_of_its_own_release() {
    let wall = frame_in("IFC2X3", IFC2X3_DATA, "#33").unwrap();
    assert_frame(
        &wall,
        [1.0, 3.0, 0.5],
        [0.0, 1.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
    );
}
