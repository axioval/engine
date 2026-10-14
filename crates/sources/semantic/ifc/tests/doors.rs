//! Door leaves come from the operation type, the panel properties and the
//! placement, in metres; the lining thickness and each leaf's depth from
//! the lining and panel properties.
#![allow(missing_docs, clippy::format_push_string)]

use axioval_engine::{
    DoorLeaves, DoorLeavesError, HingeSide, LeafMotion, LeafPosition, ObjectFrameServiceHandle,
    UnplacedLeaf,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{ObjectId, SourceId};

/// One panel set: `PanelOperation`, `PanelPosition` and `PanelWidth` (a
/// ratio, or `$`), 40 mm deep.
struct Panel(&'static str, &'static str, &'static str);

/// An IFC4 door #20 in millimetres, `width` wide, typed by #21 with
/// `operation` and the panels, placed at (1, 2, 0) m with `axis` and
/// `reference` as its placement's axis and reference direction. The type
/// also carries a lining 50 mm thick.
struct Door {
    operation: &'static str,
    width: &'static str,
    panels: Vec<Panel>,
    axis: &'static str,
    reference: &'static str,
    /// Written in IFC2X3, typed by an `IfcDoorStyle`, rather than IFC4.
    ifc2x3: bool,
}

impl Door {
    fn new(operation: &'static str, panels: Vec<Panel>) -> Self {
        Self {
            operation,
            width: "900.",
            panels,
            axis: "(0.,0.,1.)",
            reference: "(1.,0.,0.)",
            ifc2x3: false,
        }
    }

    /// The same door written in IFC2X3.
    fn in_ifc2x3(mut self) -> Self {
        self.ifc2x3 = true;
        self
    }

    fn step(&self) -> String {
        // IFC2X3 has no door `PredefinedType`, `OperationType` and
        // `UserDefinedOperationType`, no wall `PredefinedType` and no
        // lining-to-panel offsets.
        let (door_tail, wall_tail, lining_tail) = if self.ifc2x3 {
            ("", "", "")
        } else {
            (",$,$,$", ",$", ",$,$")
        };
        let mut data = format!(
            "#1=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#2=IFCUNITASSIGNMENT((#1));
#3=IFCPROJECT('0000000000000000000003',$,'P',$,$,$,$,$,#2);
#10=IFCCARTESIANPOINT((1000.,2000.,0.));
#11=IFCDIRECTION({axis});
#12=IFCDIRECTION({reference});
#13=IFCAXIS2PLACEMENT3D(#10,#11,#12);
#14=IFCLOCALPLACEMENT($,#13);
#20=IFCDOOR('000000000000000000000D',$,'Door',$,$,#14,$,$,2100.,{width}{door_tail});
#22=IFCRELDEFINESBYTYPE('000000000000000000000R',$,$,$,(#20),#21);
#23=IFCWALL('000000000000000000000W',$,'Wall',$,$,#14,$,${wall_tail});
#40=IFCDOORLININGPROPERTIES('000000000000000000000L',$,$,$,100.,50.,$,20.,$,$,$,$,$,$,${lining_tail});
",
            axis = self.axis,
            reference = self.reference,
            width = self.width,
        );
        let mut sets = vec!["#40".to_owned()];
        for (index, Panel(operation, position, ratio)) in self.panels.iter().enumerate() {
            let id = 30 + index;
            sets.push(format!("#{id}"));
            data.push_str(&format!(
                "#{id}=IFCDOORPANELPROPERTIES('00000000000000000000P{index}',$,$,$,40.,.{operation}.,{ratio},.{position}.,$);\n"
            ));
        }
        if self.ifc2x3 {
            data.push_str(&format!(
                "#21=IFCDOORSTYLE('000000000000000000000T',$,'Type',$,$,({}),$,$,.{}.,.NOTDEFINED.,.F.,.F.);\n",
                sets.join(","),
                self.operation
            ));
        } else {
            data.push_str(&format!(
                "#21=IFCDOORTYPE('000000000000000000000T',$,'Type',$,$,({}),$,$,$,.DOOR.,.{}.,.F.,$);\n",
                sets.join(","),
                self.operation
            ));
        }
        let schema = if self.ifc2x3 { "IFC2X3" } else { "IFC4" };
        format!(
            "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('{schema}'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
        )
    }

    fn leaves_of(&self, local: &str) -> Result<DoorLeaves, DoorLeavesError> {
        let session = import_ifc_session("doors.ifc", self.step().as_bytes()).unwrap();
        let object = ObjectId::new(SourceId::new("ifc-step", "doors.ifc").unwrap(), local).unwrap();
        session
            .service::<ObjectFrameServiceHandle>()
            .expect("the IFC session registers object frames")
            .leaves(&object)
    }

    fn leaves(&self) -> DoorLeaves {
        self.leaves_of("#20").expect("the door's leaves are stated")
    }
}

fn close(actual: [f64; 3], expected: [f64; 3]) {
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|(a, e)| (a - e).abs() <= 1e-9),
        "{actual:?} != {expected:?}"
    );
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() <= 1e-9, "{actual} != {expected}");
}

#[test]
fn a_single_swing_left_leaf_hinges_at_the_low_jamb_and_opens_to_local_y() {
    let door = Door::new("SINGLE_SWING_LEFT", vec![Panel("SWINGING", "LEFT", "$")]);
    let leaves = door.leaves();
    assert_eq!(leaves.operation(), "SINGLE_SWING_LEFT");
    near(leaves.overall_width_metres(), 0.9);
    near(leaves.lining_thickness_metres().unwrap(), 0.05);
    assert!(leaves.evidence().exact);
    assert!(
        leaves
            .evidence()
            .locator
            .contains(":door-operation:#20:SINGLE_SWING_LEFT:panels=#30:lining=#40"),
        "{}",
        leaves.evidence().locator
    );
    let [leaf] = leaves.leaves() else {
        panic!("one leaf")
    };
    assert_eq!(leaf.motion(), LeafMotion::Swing);
    assert_eq!(leaf.position(), LeafPosition::Left);
    assert_eq!(leaf.hinge_side(), Some(HingeSide::Left));
    near(leaf.width_metres(), 0.9);
    near(leaf.depth_metres().unwrap(), 0.04);
    close(leaf.origin(), [1.0, 2.0, 0.0]);
    close(leaf.opening().components(), [0.0, 1.0, 0.0]);
    assert!(!leaf.is_mirrored());
    let sector = leaf.swing().unwrap();
    close(sector.hinge(), [1.0, 2.0, 0.0]);
    close(sector.closed().components(), [1.0, 0.0, 0.0]);
    close(sector.open().components(), [0.0, 1.0, 0.0]);
    near(sector.radius_metres(), 0.9);
    assert!(!sector.is_double_acting());
}

#[test]
fn a_single_swing_right_leaf_hinges_at_the_far_jamb() {
    let door = Door::new("SINGLE_SWING_RIGHT", vec![Panel("SWINGING", "RIGHT", "$")]);
    let leaves = door.leaves();
    let leaf = &leaves.leaves()[0];
    assert_eq!(leaf.hinge_side(), Some(HingeSide::Right));
    let sector = leaf.swing().unwrap();
    close(sector.hinge(), [1.9, 2.0, 0.0]);
    close(sector.closed().components(), [-1.0, 0.0, 0.0]);
    close(sector.open().components(), [0.0, 1.0, 0.0]);
}

#[test]
fn a_door_turned_over_opens_the_other_way_with_the_hinge_on_the_other_side() {
    // Axis -z: local y is world -y, so seen from above the left-hinged leaf
    // hangs on the right of its opening direction.
    let mut door = Door::new("SINGLE_SWING_LEFT", vec![Panel("SWINGING", "LEFT", "$")]);
    door.axis = "(0.,0.,-1.)";
    let leaves = door.leaves();
    let leaf = &leaves.leaves()[0];
    assert_eq!(leaf.hinge_side(), Some(HingeSide::Right));
    close(leaf.opening().components(), [0.0, -1.0, 0.0]);
    let sector = leaf.swing().unwrap();
    close(sector.hinge(), [1.0, 2.0, 0.0]);
    close(sector.open().components(), [0.0, -1.0, 0.0]);
    assert!(sector.is_horizontal());
}

#[test]
fn a_double_door_hinges_each_leaf_at_its_jamb() {
    let door = Door::new(
        "DOUBLE_DOOR_SINGLE_SWING",
        vec![
            Panel("SWINGING", "LEFT", "0.6"),
            Panel("SWINGING", "RIGHT", "0.4"),
        ],
    );
    let leaves = door.leaves();
    let [left, right] = leaves.leaves() else {
        panic!("two leaves")
    };
    assert_eq!(left.hinge_side(), Some(HingeSide::Left));
    assert_eq!(right.hinge_side(), Some(HingeSide::Right));
    near(left.width_metres(), 0.54);
    near(right.width_metres(), 0.36);
    close(left.swing().unwrap().hinge(), [1.0, 2.0, 0.0]);
    close(right.swing().unwrap().hinge(), [1.9, 2.0, 0.0]);
    close(right.origin(), [1.54, 2.0, 0.0]);
    assert_eq!(leaves.hinged().count(), 2);
}

#[test]
fn a_double_acting_leaf_sweeps_a_half_disc() {
    let door = Door::new(
        "DOUBLE_SWING_RIGHT",
        vec![Panel("DOUBLE_ACTING", "RIGHT", "$")],
    );
    let leaves = door.leaves();
    let leaf = &leaves.leaves()[0];
    assert_eq!(leaf.motion(), LeafMotion::DoubleSwing);
    let sector = leaf.swing().unwrap();
    assert!(sector.is_double_acting());
    close(sector.closed().components(), [-1.0, 0.0, 0.0]);
    close(sector.direction_at(0.0), [0.0, -1.0, 0.0]);
    close(sector.direction_at(sector.sweep()), [0.0, 1.0, 0.0]);
}

#[test]
fn a_sliding_leaf_has_no_hinge_and_no_sector() {
    let door = Door::new("SLIDING_TO_LEFT", vec![Panel("SLIDING", "LEFT", "$")]);
    let leaves = door.leaves();
    let leaf = &leaves.leaves()[0];
    let LeafMotion::Slide(direction) = leaf.motion() else {
        panic!("a sliding leaf: {:?}", leaf.motion())
    };
    close(direction.components(), [-1.0, 0.0, 0.0]);
    assert_eq!(leaf.hinge_side(), None);
    assert!(leaf.swing().is_none());
    assert_eq!(leaves.hinged().count(), 0);
}

#[test]
fn what_the_source_does_not_state_is_refused_never_defaulted() {
    let undefined = Door::new("NOTDEFINED", vec![Panel("SWINGING", "LEFT", "$")]);
    assert!(matches!(
        undefined.leaves_of("#20"),
        Err(DoorLeavesError::Refused(_))
    ));
    let bare = Door::new("SINGLE_SWING_LEFT", vec![]);
    assert!(matches!(
        bare.leaves_of("#20"),
        Err(DoorLeavesError::NotStated(_))
    ));
    let mut unmeasured = Door::new("SINGLE_SWING_LEFT", vec![Panel("SWINGING", "LEFT", "$")]);
    unmeasured.width = "$";
    assert!(matches!(
        unmeasured.leaves_of("#20"),
        Err(DoorLeavesError::NotStated(_))
    ));
    let mismatched = Door::new("SINGLE_SWING_LEFT", vec![Panel("SLIDING", "LEFT", "$")]);
    assert!(matches!(
        mismatched.leaves_of("#20"),
        Err(DoorLeavesError::Refused(_))
    ));
    let door = Door::new("SINGLE_SWING_LEFT", vec![Panel("SWINGING", "LEFT", "$")]);
    assert!(matches!(
        door.leaves_of("#23"),
        Err(DoorLeavesError::NotADoor(_))
    ));
    // The door type is a session object, and no door: it has no leaves,
    // never a failure to read them.
    assert!(matches!(
        door.leaves_of("#21"),
        Err(DoorLeavesError::NotADoor(_))
    ));
    assert!(matches!(
        door.leaves_of("#99"),
        Err(DoorLeavesError::UnknownObject(_))
    ));
}

/// The leaf a single-leaf door states without its place along the opening
/// (#378).
fn unplaced(door: &Door) -> UnplacedLeaf {
    match door.leaves_of("#20") {
        Err(DoorLeavesError::Unplaced(leaf)) => *leaf,
        other => panic!("an unplaced leaf: {other:?}"),
    }
}

#[test]
fn a_single_panel_at_middle_is_measured_without_its_place() {
    for ifc2x3 in [false, true] {
        let mut door = Door::new(
            "SINGLE_SWING_LEFT",
            vec![Panel("SWINGING", "MIDDLE", "0.96")],
        );
        door.ifc2x3 = ifc2x3;
        let leaf = unplaced(&door);
        assert_eq!(leaf.door().local_id, "#20");
        assert_eq!(leaf.position(), LeafPosition::Middle);
        assert_eq!(leaf.motion(), LeafMotion::Swing);
        near(leaf.overall_width_metres(), 0.9);
        near(leaf.width_metres(), 0.96 * 0.9);
        near(leaf.lining_thickness_metres().unwrap(), 0.05);
        near(leaf.depth_metres().unwrap(), 0.04);
        assert!(leaf.evidence().exact);
        assert!(
            leaf.evidence().locator.contains(
                ":door-operation:#20:unplaced:panel=#30:SWINGING:MIDDLE:width=0.96:lining=#40"
            ),
            "{}",
            leaf.evidence().locator
        );
        // Everything that needs the leaf's place reads the refusal as it
        // was.
        let refusal = DoorLeavesError::Unplaced(Box::new(leaf)).to_string();
        assert!(
            refusal.starts_with("the door's leaves cannot be derived exactly: UnplacedPanel"),
            "{refusal}"
        );
    }
    let undefined = Door::new(
        "DOUBLE_SWING_RIGHT",
        vec![Panel("DOUBLE_ACTING", "NOTDEFINED", "0.8")],
    )
    .in_ifc2x3();
    let leaf = unplaced(&undefined);
    assert_eq!(leaf.position(), LeafPosition::NotDefined);
    assert_eq!(leaf.motion(), LeafMotion::DoubleSwing);
    near(leaf.width_metres(), 0.72);
}

#[test]
fn a_single_panel_at_left_narrower_than_the_opening_is_placed_as_before() {
    for ifc2x3 in [false, true] {
        let mut door = Door::new("SINGLE_SWING_LEFT", vec![Panel("SWINGING", "LEFT", "0.96")]);
        door.ifc2x3 = ifc2x3;
        let leaves = door.leaves();
        let [leaf] = leaves.leaves() else {
            panic!("one leaf")
        };
        assert_eq!(leaf.position(), LeafPosition::Left);
        near(leaf.width_metres(), 0.96 * 0.9);
        close(leaf.origin(), [1.0, 2.0, 0.0]);
        assert!(leaf.swing().is_some());
    }
}

#[test]
fn panels_that_cannot_be_placed_stay_refused() {
    for ifc2x3 in [false, true] {
        let refused = |mut door: Door| {
            door.ifc2x3 = ifc2x3;
            match door.leaves_of("#20") {
                Err(DoorLeavesError::Refused(why)) => why,
                other => panic!("refused: {other:?}"),
            }
        };
        // Two panels, one of them without a place: neither can be placed.
        let why = refused(Door::new(
            "DOUBLE_DOOR_SINGLE_SWING",
            vec![
                Panel("SWINGING", "LEFT", "0.5"),
                Panel("SWINGING", "MIDDLE", "0.5"),
            ],
        ));
        assert!(why.contains("PanelMismatch"), "{why}");
        // A ratio above 1 is invalid wherever the panel stands.
        let why = refused(Door::new(
            "SINGLE_SWING_LEFT",
            vec![Panel("SWINGING", "MIDDLE", "1.2")],
        ));
        assert!(why.contains("InvalidPanelWidth"), "{why}");
        // A sliding leaf leaves the opening along it, so where it goes
        // depends on where it stands.
        let why = refused(Door::new(
            "SLIDING_TO_LEFT",
            vec![Panel("SLIDING", "MIDDLE", "0.96")],
        ));
        assert!(why.contains("UnplacedPanel"), "{why}");
    }
}
