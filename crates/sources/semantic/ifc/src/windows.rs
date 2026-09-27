//! Window leaves from an `IfcWindow`'s partitioning and panel properties.
//!
//! The panels come from `openbim-ifc`'s `window_operation`
//! (openbimrs/ifc#170), the window counterpart of `door_operation`: it
//! joins the window's placement with its partitioning (the occurrence's
//! `PartitioningType`, its `IfcWindowType`'s or, in IFC2X3, its
//! `IfcWindowStyle.OperationType`), its `IfcWindowPanelProperties` and the
//! mullion and transom offsets of its `IfcWindowLiningProperties`, and
//! cites the IFC documentation for every convention: panels tile the
//! placement's XZ plane and open towards local +y. It is never re-derived
//! here.
//!
//! Each panel is a leaf with its height and, for a top-, bottom-hung or
//! tilt-and-turn panel, its tilt sector. Added here, from the same exact
//! property resolver as the doors: the lining thickness
//! (`IfcWindowLiningProperties.LiningThickness`). A panel's depth is its
//! `FrameDepth` as upstream reads it.

use axioval_engine::{
    DoorLeaf, DoorLeaves, DoorLeavesError, HingeSide, LeafMotion, LeafPosition, SwingSector,
};
use axioval_ir::{Evidence, ObjectId};
use ifc_model::{EntityId, Model};
use openbim_ifc::{
    Sector, Side, WindowOperationError, WindowPanel, WindowPanelMotion, WindowPanelPosition,
    WindowPartitioning, window_operation,
};

use crate::doors::{direction, lining};

/// The partitioning's name as `IfcWindowTypePartitioningEnum` spells it.
fn partitioning_name(partitioning: WindowPartitioning) -> Result<&'static str, DoorLeavesError> {
    use WindowPartitioning as P;
    Ok(match partitioning {
        P::SinglePanel => "SINGLE_PANEL",
        P::DoublePanelVertical => "DOUBLE_PANEL_VERTICAL",
        P::DoublePanelHorizontal => "DOUBLE_PANEL_HORIZONTAL",
        P::TriplePanelVertical => "TRIPLE_PANEL_VERTICAL",
        P::TriplePanelHorizontal => "TRIPLE_PANEL_HORIZONTAL",
        P::TriplePanelBottom => "TRIPLE_PANEL_BOTTOM",
        P::TriplePanelTop => "TRIPLE_PANEL_TOP",
        P::TriplePanelLeft => "TRIPLE_PANEL_LEFT",
        P::TriplePanelRight => "TRIPLE_PANEL_RIGHT",
        other => {
            return Err(DoorLeavesError::Refused(format!(
                "partitioning {other:?} is not mapped"
            )));
        }
    })
}

/// Why the upstream derivation refused, in the contract's terms; `None`
/// when the object is not a window.
fn operation_error(error: &WindowOperationError) -> Option<DoorLeavesError> {
    use WindowOperationError as E;
    let detail = format!("{error:?}");
    Some(match error {
        E::NotAWindow { .. } => return None,
        E::MissingPartitioningType { .. }
        | E::MissingOverallWidth { .. }
        | E::MissingOverallHeight { .. }
        | E::NoPanelProperties { .. }
        | E::NoLiningProperties { .. }
        | E::MissingSplit { .. } => DoorLeavesError::NotStated(detail),
        E::RefusedPartitioning { .. }
        | E::RefusedPanelOperation { .. }
        | E::ConflictingPartitioningType { .. }
        | E::UnsupportedTypeObject { .. }
        | E::PanelCount { .. }
        | E::PanelMismatch { .. }
        | E::LiningCount { .. }
        | E::InvalidSplit { .. }
        | E::NonRigidPlacement { .. }
        | E::NoPlanHandedness { .. } => DoorLeavesError::Refused(detail),
        _ => DoorLeavesError::Unreadable(detail),
    })
}

/// An upstream sector as the contract's: from the closed panel to the panel
/// open at right angles.
fn sector(sector: &Sector) -> Result<SwingSector, DoorLeavesError> {
    SwingSector::try_new(
        sector.center,
        sector.radius,
        direction(sector.start)?,
        direction(sector.end())?,
        false,
    )
}

fn leaf(panel: &WindowPanel) -> Result<DoorLeaf, DoorLeavesError> {
    let [x, y, z] = panel.frame_world.basis;
    let (along, opening, up) = (direction(x)?, direction(y)?, direction(z)?);
    let position = match panel.position {
        WindowPanelPosition::Left => LeafPosition::Left,
        WindowPanelPosition::Middle => LeafPosition::Middle,
        WindowPanelPosition::Right => LeafPosition::Right,
        WindowPanelPosition::Bottom => LeafPosition::Bottom,
        WindowPanelPosition::Top => LeafPosition::Top,
        WindowPanelPosition::NotDefined => LeafPosition::NotDefined,
        other => {
            return Err(DoorLeavesError::Refused(format!(
                "panel position {other:?} is not mapped"
            )));
        }
    };
    let motion = match panel.motion {
        WindowPanelMotion::Swing => LeafMotion::Swing,
        WindowPanelMotion::TiltAndTurn => LeafMotion::TiltAndTurn,
        WindowPanelMotion::TopHung | WindowPanelMotion::BottomHung => LeafMotion::Tilt,
        WindowPanelMotion::Slide { along: slide } => LeafMotion::Slide(direction(slide)?),
        WindowPanelMotion::Removable => LeafMotion::Removable,
        WindowPanelMotion::Fixed => LeafMotion::Fixed,
        other => {
            return Err(DoorLeavesError::Refused(format!(
                "panel motion {other:?} is not mapped"
            )));
        }
    };
    let hinge_side = panel.hinge_side.map(|side| match side {
        Side::Left => HingeSide::Left,
        Side::Right => HingeSide::Right,
    });
    let swing = panel.swing.as_ref().map(sector).transpose()?;
    let leaf = DoorLeaf::try_new(
        position,
        motion,
        panel.frame_world.origin,
        along,
        opening,
        up,
        panel.width,
        panel.frame_depth.filter(|depth| *depth > 0.0),
        hinge_side,
        swing,
    )?
    .with_height(panel.height)?;
    match &panel.tilt {
        Some(tilt) => leaf.with_tilt(sector(tilt)?),
        None => Ok(leaf),
    }
}

/// The panels of `window` (entity `id`) as leaves, cited under
/// `fingerprint`; `NotADoor` when it is not a window either.
pub(crate) fn window_leaves(
    model: &Model,
    fingerprint: &str,
    window: &ObjectId,
    id: EntityId,
) -> Result<DoorLeaves, DoorLeavesError> {
    let operation = window_operation(model, id).map_err(|error| {
        operation_error(&error).unwrap_or_else(|| DoorLeavesError::NotADoor(window.clone()))
    })?;
    let name = partitioning_name(operation.partitioning)?;
    let (lining_thickness, lining_set) = lining(model, id, "IfcWindowLiningProperties")?;
    let leaves = operation
        .panels
        .iter()
        .map(leaf)
        .collect::<Result<Vec<_>, _>>()?;
    let sets = operation
        .panels
        .iter()
        .map(|panel| format!("#{}", panel.panel_set.0))
        .collect::<Vec<_>>()
        .join(",");
    let lining = lining_set.map_or(String::new(), |set| format!(":lining=#{}", set.0));
    let evidence = Evidence::exact(
        window.source.clone(),
        format!("ifc:{fingerprint}:window-operation:{id}:{name}:panels={sets}{lining}"),
    );
    DoorLeaves::try_new(
        window.clone(),
        name,
        operation.overall_width,
        lining_thickness,
        leaves,
        evidence,
    )
}
