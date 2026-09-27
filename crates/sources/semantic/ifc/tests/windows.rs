//! Window panels are leaves: they come from the partitioning, the panel
//! properties, the lining's mullion and transom offsets and the placement,
//! in metres, each with its height and, when it tilts, its tilt sector.
#![allow(missing_docs, clippy::format_push_string)]

use axioval_engine::{
    DoorLeaves, DoorLeavesError, HingeSide, LeafMotion, LeafPosition, ObjectFrameServiceHandle,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{ObjectId, SourceId};

/// One panel set: `OperationType` and `PanelPosition`, its frame 60 mm
/// deep and 50 mm thick.
struct Panel(&'static str, &'static str);

/// An IFC4 window #20 in millimetres, 800 mm wide and 1200 mm high, typed
/// by #21 with `partitioning` and the panels, placed at (1, 2, 0.9) m with
/// `axis` and `reference` as its placement's axis and reference direction.
/// With `lining`, the type also carries a lining 40 mm thick whose first
/// mullion and transom lie half way.
struct Window {
    schema: &'static str,
    partitioning: &'static str,
    width: &'static str,
    panels: Vec<Panel>,
    lining: bool,
    axis: &'static str,
    reference: &'static str,
}

impl Window {
    fn new(partitioning: &'static str, panels: Vec<Panel>) -> Self {
        Self {
            schema: "IFC4",
            partitioning,
            width: "800.",
            panels,
            lining: false,
            axis: "(0.,0.,1.)",
            reference: "(1.,0.,0.)",
        }
    }

    fn lined(mut self) -> Self {
        self.lining = true;
        self
    }

    fn step(&self) -> String {
        let ifc4 = self.schema == "IFC4";
        let mut data = format!(
            "#1=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#2=IFCUNITASSIGNMENT((#1));
#3=IFCPROJECT('0000000000000000000003',$,'P',$,$,$,$,$,#2);
#10=IFCCARTESIANPOINT((1000.,2000.,900.));
#11=IFCDIRECTION({axis});
#12=IFCDIRECTION({reference});
#13=IFCAXIS2PLACEMENT3D(#10,#11,#12);
#14=IFCLOCALPLACEMENT($,#13);
#22=IFCRELDEFINESBYTYPE('000000000000000000000R',$,$,$,(#20),#21);
",
            axis = self.axis,
            reference = self.reference,
        );
        if ifc4 {
            data.push_str(&format!(
                "#20=IFCWINDOW('000000000000000000000D',$,'Window',$,$,#14,$,$,1200.,{},.WINDOW.,$,$);
#23=IFCWALL('000000000000000000000W',$,'Wall',$,$,#14,$,$,$);
",
                self.width
            ));
        } else {
            data.push_str(&format!(
                "#20=IFCWINDOW('000000000000000000000D',$,'Window',$,$,#14,$,$,1200.,{});\n",
                self.width
            ));
        }
        let mut sets = Vec::new();
        if self.lining {
            sets.push("#40".to_owned());
            data.push_str(
                "#40=IFCWINDOWLININGPROPERTIES('000000000000000000000L',$,$,$,100.,40.,$,$,0.5,$,0.5,$,$",
            );
            data.push_str(if ifc4 { ",$,$,$);\n" } else { ");\n" });
        }
        for (index, Panel(operation, position)) in self.panels.iter().enumerate() {
            let id = 30 + index;
            sets.push(format!("#{id}"));
            data.push_str(&format!(
                "#{id}=IFCWINDOWPANELPROPERTIES('00000000000000000000P{index}',$,$,$,.{operation}.,.{position}.,60.,50.,$);\n"
            ));
        }
        let sets = if sets.is_empty() {
            "$".to_owned()
        } else {
            format!("({})", sets.join(","))
        };
        if ifc4 {
            data.push_str(&format!(
                "#21=IFCWINDOWTYPE('000000000000000000000T',$,'Type',$,$,{sets},$,$,$,.WINDOW.,.{}.,.F.,$);\n",
                self.partitioning
            ));
        } else {
            data.push_str(&format!(
                "#21=IFCWINDOWSTYLE('000000000000000000000T',$,'Style',$,$,{sets},$,$,.WOOD.,.{}.,.F.,.F.);\n",
                self.partitioning
            ));
        }
        format!(
            "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('{}'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n",
            self.schema
        )
    }

    fn leaves_of(&self, local: &str) -> Result<DoorLeaves, DoorLeavesError> {
        let session = import_ifc_session("windows.ifc", self.step().as_bytes()).unwrap();
        let object =
            ObjectId::new(SourceId::new("ifc-step", "windows.ifc").unwrap(), local).unwrap();
        session
            .service::<ObjectFrameServiceHandle>()
            .expect("the IFC session registers object frames")
            .leaves(&object)
    }

    fn leaves(&self) -> DoorLeaves {
        self.leaves_of("#20")
            .expect("the window's panels are stated")
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
fn a_casement_hinged_left_swings_about_its_low_jamb_at_sill_height() {
    let window = Window::new("SINGLE_PANEL", vec![Panel("SIDEHUNGLEFTHAND", "MIDDLE")]);
    let leaves = window.leaves();
    assert_eq!(leaves.operation(), "SINGLE_PANEL");
    near(leaves.overall_width_metres(), 0.8);
    assert_eq!(leaves.lining_thickness_metres(), None);
    assert!(leaves.evidence().exact);
    assert!(
        leaves
            .evidence()
            .locator
            .contains(":window-operation:#20:SINGLE_PANEL:panels=#30"),
        "{}",
        leaves.evidence().locator
    );
    let [leaf] = leaves.leaves() else {
        panic!("one panel")
    };
    assert_eq!(leaf.motion(), LeafMotion::Swing);
    assert_eq!(leaf.position(), LeafPosition::Middle);
    assert_eq!(leaf.hinge_side(), Some(HingeSide::Left));
    near(leaf.width_metres(), 0.8);
    near(leaf.height_metres().unwrap(), 1.2);
    near(leaf.depth_metres().unwrap(), 0.06);
    close(leaf.origin(), [1.0, 2.0, 0.9]);
    close(leaf.opening().components(), [0.0, 1.0, 0.0]);
    let sector = leaf.swing().unwrap();
    close(sector.hinge(), [1.0, 2.0, 0.9]);
    close(sector.closed().components(), [1.0, 0.0, 0.0]);
    close(sector.open().components(), [0.0, 1.0, 0.0]);
    near(sector.radius_metres(), 0.8);
    assert!(sector.is_horizontal());
    assert!(leaf.tilt().is_none());
    assert_eq!(leaves.hinged().count(), 1);
}

#[test]
fn a_casement_hinged_right_swings_about_its_far_jamb() {
    let window = Window::new("SINGLE_PANEL", vec![Panel("SIDEHUNGRIGHTHAND", "MIDDLE")]);
    let leaves = window.leaves();
    let leaf = &leaves.leaves()[0];
    assert_eq!(leaf.hinge_side(), Some(HingeSide::Right));
    let sector = leaf.swing().unwrap();
    close(sector.hinge(), [1.8, 2.0, 0.9]);
    close(sector.closed().components(), [-1.0, 0.0, 0.0]);
    close(sector.open().components(), [0.0, 1.0, 0.0]);
}

#[test]
fn a_window_turned_over_opens_the_other_way_with_the_hinge_on_the_other_side() {
    // Axis -z: local y is world -y, so seen from above the left-hinged
    // casement hangs on the right of its opening direction.
    let mut window = Window::new("SINGLE_PANEL", vec![Panel("SIDEHUNGLEFTHAND", "MIDDLE")]);
    window.axis = "(0.,0.,-1.)";
    let leaves = window.leaves();
    let leaf = &leaves.leaves()[0];
    assert_eq!(leaf.hinge_side(), Some(HingeSide::Right));
    close(leaf.opening().components(), [0.0, -1.0, 0.0]);
    close(leaf.up().components(), [0.0, 0.0, -1.0]);
    let sector = leaf.swing().unwrap();
    close(sector.hinge(), [1.0, 2.0, 0.9]);
    close(sector.open().components(), [0.0, -1.0, 0.0]);
    assert!(sector.is_horizontal());
}

#[test]
fn hung_and_tilt_and_turn_panels_tilt_on_their_top_or_bottom_hinge() {
    let bottom = Window::new("SINGLE_PANEL", vec![Panel("BOTTOMHUNG", "MIDDLE")]).leaves();
    let leaf = &bottom.leaves()[0];
    assert_eq!(leaf.motion(), LeafMotion::Tilt);
    assert_eq!(leaf.hinge_side(), None);
    assert!(leaf.swing().is_none());
    let tilt = leaf.tilt().unwrap();
    close(tilt.hinge(), [1.0, 2.0, 0.9]);
    close(tilt.closed().components(), [0.0, 0.0, 1.0]);
    close(tilt.open().components(), [0.0, 1.0, 0.0]);
    near(tilt.radius_metres(), 1.2);
    assert!(!tilt.is_horizontal());
    assert_eq!(bottom.hinged().count(), 0);
    let top = Window::new("SINGLE_PANEL", vec![Panel("TOPHUNG", "MIDDLE")]).leaves();
    let tilt = top.leaves()[0].tilt().unwrap();
    close(tilt.hinge(), [1.0, 2.0, 2.1]);
    close(tilt.closed().components(), [0.0, 0.0, -1.0]);
    let turning = Window::new(
        "SINGLE_PANEL",
        vec![Panel("TILTANDTURNRIGHTHAND", "MIDDLE")],
    )
    .leaves();
    let leaf = &turning.leaves()[0];
    assert_eq!(leaf.motion(), LeafMotion::TiltAndTurn);
    assert_eq!(leaf.hinge_side(), Some(HingeSide::Right));
    close(leaf.swing().unwrap().hinge(), [1.8, 2.0, 0.9]);
    close(leaf.tilt().unwrap().hinge(), [1.0, 2.0, 0.9]);
    assert_eq!(turning.hinged().count(), 1);
}

#[test]
fn a_fixed_panel_neither_swings_nor_tilts() {
    let leaves = Window::new("SINGLE_PANEL", vec![Panel("FIXEDCASEMENT", "MIDDLE")]).leaves();
    let leaf = &leaves.leaves()[0];
    assert_eq!(leaf.motion(), LeafMotion::Fixed);
    assert_eq!(leaf.hinge_side(), None);
    assert!(leaf.swing().is_none() && leaf.tilt().is_none());
    assert_eq!(leaves.hinged().count(), 0);
}

#[test]
fn a_two_panel_window_splits_at_its_mullion() {
    let window = Window::new(
        "DOUBLE_PANEL_VERTICAL",
        vec![
            Panel("SIDEHUNGLEFTHAND", "LEFT"),
            Panel("SIDEHUNGRIGHTHAND", "RIGHT"),
        ],
    )
    .lined();
    let leaves = window.leaves();
    near(leaves.lining_thickness_metres().unwrap(), 0.04);
    assert!(
        leaves
            .evidence()
            .locator
            .contains(":window-operation:#20:DOUBLE_PANEL_VERTICAL:panels=#30,#31:lining=#40"),
        "{}",
        leaves.evidence().locator
    );
    let [left, right] = leaves.leaves() else {
        panic!("two panels")
    };
    assert_eq!(left.position(), LeafPosition::Left);
    assert_eq!(right.position(), LeafPosition::Right);
    near(left.width_metres(), 0.4);
    near(right.width_metres(), 0.4);
    close(right.origin(), [1.4, 2.0, 0.9]);
    close(left.swing().unwrap().hinge(), [1.0, 2.0, 0.9]);
    close(right.swing().unwrap().hinge(), [1.8, 2.0, 0.9]);
    assert_eq!(leaves.hinged().count(), 2);
}

#[test]
fn an_ifc2x3_window_reads_its_partitioning_from_its_style() {
    let mut window = Window::new("SINGLE_PANEL", vec![Panel("SIDEHUNGLEFTHAND", "MIDDLE")]);
    window.schema = "IFC2X3";
    let leaves = window.leaves();
    assert_eq!(leaves.operation(), "SINGLE_PANEL");
    assert_eq!(leaves.leaves()[0].hinge_side(), Some(HingeSide::Left));
}

#[test]
fn what_the_source_does_not_state_is_refused_never_defaulted() {
    let pivot = Window::new("SINGLE_PANEL", vec![Panel("PIVOTHORIZONTAL", "MIDDLE")]);
    assert!(matches!(
        pivot.leaves_of("#20"),
        Err(DoorLeavesError::Refused(_))
    ));
    let undefined = Window::new("NOTDEFINED", vec![Panel("SIDEHUNGLEFTHAND", "MIDDLE")]);
    assert!(matches!(
        undefined.leaves_of("#20"),
        Err(DoorLeavesError::Refused(_))
    ));
    let bare = Window::new("SINGLE_PANEL", vec![]);
    assert!(matches!(
        bare.leaves_of("#20"),
        Err(DoorLeavesError::NotStated(_))
    ));
    let mut unmeasured = Window::new("SINGLE_PANEL", vec![Panel("SIDEHUNGLEFTHAND", "MIDDLE")]);
    unmeasured.width = "$";
    assert!(matches!(
        unmeasured.leaves_of("#20"),
        Err(DoorLeavesError::NotStated(_))
    ));
    // Two panels need the lining's mullion to be split.
    let unsplit = Window::new(
        "DOUBLE_PANEL_VERTICAL",
        vec![
            Panel("SIDEHUNGLEFTHAND", "LEFT"),
            Panel("SIDEHUNGRIGHTHAND", "RIGHT"),
        ],
    );
    assert!(matches!(
        unsplit.leaves_of("#20"),
        Err(DoorLeavesError::NotStated(_))
    ));
    let window = Window::new("SINGLE_PANEL", vec![Panel("SIDEHUNGLEFTHAND", "MIDDLE")]);
    assert!(matches!(
        window.leaves_of("#23"),
        Err(DoorLeavesError::NotADoor(_))
    ));
    assert!(matches!(
        window.leaves_of("#99"),
        Err(DoorLeavesError::UnknownObject(_))
    ));
}
