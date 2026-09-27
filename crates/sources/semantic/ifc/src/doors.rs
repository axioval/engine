//! Door leaves from an `IfcDoor`'s operation type and panel properties.
//!
//! The leaves come from `openbim-ifc`'s `door_operation`, which joins the
//! door's placement with its `OperationType` and `IfcDoorPanelProperties`
//! and cites the IFC4 documentation for every convention it applies:
//! leaves lie on the placement's x axis and a swinging leaf opens towards
//! local +y. That derivation lives in the upstream facade because it joins
//! two sibling crates (`ifc-geometry` and `ifc-properties`); it is never
//! re-derived here.
//!
//! Added here, from the same exact property resolver: the lining thickness
//! (`IfcDoorLiningProperties.LiningThickness`) and each leaf's depth
//! (`IfcDoorPanelProperties.PanelDepth`), in metres through the exact
//! project length unit. Like the panels, the occurrence's own lining set
//! governs, else its type's; several are refused.

use axioval_engine::{
    DoorLeaf, DoorLeaves, DoorLeavesError, HingeSide, LeafMotion, LeafPosition, MetricDirection,
    SwingSector,
};
use axioval_ir::{Evidence, ObjectId};
use ifc_model::{EntityId, Model};
use ifc_properties::{
    ExactPredefinedSet, ExactPropertyError, ExactSource, ExactValue, exact_predefined_sets,
    exact_unit,
};
use openbim_ifc::{
    DoorOperation, DoorOperationError, DoorOperationType, Leaf, PanelPosition, Side, door_operation,
};

/// The operation type's name as `IfcDoorTypeOperationEnum` spells it.
fn operation_name(operation: DoorOperationType) -> Result<&'static str, DoorLeavesError> {
    use DoorOperationType as T;
    Ok(match operation {
        T::SingleSwingLeft => "SINGLE_SWING_LEFT",
        T::SingleSwingRight => "SINGLE_SWING_RIGHT",
        T::DoubleDoorSingleSwing => "DOUBLE_DOOR_SINGLE_SWING",
        T::DoubleSwingLeft => "DOUBLE_SWING_LEFT",
        T::DoubleSwingRight => "DOUBLE_SWING_RIGHT",
        T::DoubleDoorDoubleSwing => "DOUBLE_DOOR_DOUBLE_SWING",
        T::SlidingToLeft => "SLIDING_TO_LEFT",
        T::SlidingToRight => "SLIDING_TO_RIGHT",
        T::DoubleDoorSliding => "DOUBLE_DOOR_SLIDING",
        T::RollingUp => "ROLLINGUP",
        T::SwingFixedLeft => "SWING_FIXED_LEFT",
        T::SwingFixedRight => "SWING_FIXED_RIGHT",
        other => {
            return Err(DoorLeavesError::Refused(format!(
                "operation {other:?} is not mapped"
            )));
        }
    })
}

/// Why the upstream derivation refused, in the contract's terms.
fn operation_error(object: &ObjectId, error: &DoorOperationError) -> DoorLeavesError {
    use DoorOperationError as E;
    let detail = format!("{error:?}");
    match error {
        E::NotADoor { .. } => DoorLeavesError::NotADoor(object.clone()),
        E::MissingOperationType { .. }
        | E::MissingOverallWidth { .. }
        | E::NoPanelProperties { .. } => DoorLeavesError::NotStated(detail),
        E::RefusedOperation { .. }
        | E::ConflictingOperationType { .. }
        | E::UnsupportedTypeObject { .. }
        | E::PanelCount { .. }
        | E::PanelMismatch { .. }
        | E::MissingPanelWidth { .. }
        | E::InvalidPanelWidth { .. }
        | E::PanelWidthsDoNotPartition { .. }
        | E::UnplacedPanel { .. }
        | E::NonRigidPlacement { .. }
        | E::NoPlanHandedness { .. } => DoorLeavesError::Refused(detail),
        _ => DoorLeavesError::Unreadable(detail),
    }
}

fn property_error(error: &ExactPropertyError) -> DoorLeavesError {
    DoorLeavesError::Unreadable(format!("a door property cannot be read exactly: {error}"))
}

pub(crate) fn direction(vector: [f64; 3]) -> Result<MetricDirection, DoorLeavesError> {
    MetricDirection::try_new(vector)
        .map_err(|_| DoorLeavesError::InvalidLeaves("a leaf axis is degenerate".into()))
}

/// A length attribute of a predefined set in metres: `None` when unset.
fn length(
    model: &Model,
    set: &ExactPredefinedSet,
    attribute: &str,
) -> Result<Option<f64>, DoorLeavesError> {
    let Some(property) = set.attribute(attribute) else {
        return Ok(None);
    };
    let value = match &property.value {
        ExactValue::Null => return Ok(None),
        ExactValue::Real(value) => *value,
        other => {
            return Err(DoorLeavesError::Unreadable(format!(
                "{}.{attribute} of #{} is not a length: {other:?}",
                set.entity, set.set_id.0
            )));
        }
    };
    let declared = property.value_type.as_deref().ok_or_else(|| {
        DoorLeavesError::Unreadable(format!("{}.{attribute} has no declared type", set.entity))
    })?;
    let unit = exact_unit(model, &declared.to_ascii_uppercase(), None).map_err(|error| {
        DoorLeavesError::Unreadable(format!(
            "the unit of {}.{attribute} cannot be resolved exactly: {error}",
            set.entity
        ))
    })?;
    if unit.dimensions != [1, 0, 0, 0, 0, 0, 0] || unit.offset != 0.0 {
        return Err(DoorLeavesError::Unreadable(format!(
            "{}.{attribute} is not a length",
            set.entity
        )));
    }
    let metres = value * unit.scale;
    if !metres.is_finite() || metres < 0.0 {
        return Err(DoorLeavesError::Unreadable(format!(
            "{}.{attribute} is not a finite non-negative length",
            set.entity
        )));
    }
    Ok(Some(metres))
}

/// The governing sets of `entity`: the occurrence's own, else its type's.
fn governing(sets: Vec<ExactPredefinedSet>) -> Vec<ExactPredefinedSet> {
    let own: Vec<_> = sets
        .iter()
        .filter(|set| set.source == ExactSource::Occurrence)
        .cloned()
        .collect();
    if own.is_empty() { sets } else { own }
}

/// The lining thickness a door or window states in its `set` (its
/// `IfcDoorLiningProperties` or `IfcWindowLiningProperties`), and the
/// lining set it comes from.
pub(crate) fn lining(
    model: &Model,
    door: EntityId,
    set: &str,
) -> Result<(Option<f64>, Option<EntityId>), DoorLeavesError> {
    let sets =
        governing(exact_predefined_sets(model, door, set).map_err(|error| property_error(&error))?);
    match sets.as_slice() {
        [] => Ok((None, None)),
        [set] => Ok((length(model, set, "LiningThickness")?, Some(set.set_id))),
        _ => Err(DoorLeavesError::Refused(format!(
            "{} lining property sets govern the object",
            sets.len()
        ))),
    }
}

/// The stated depth of the panel set `set`.
fn panel_depth(
    model: &Model,
    panels: &[ExactPredefinedSet],
    set: EntityId,
) -> Result<Option<f64>, DoorLeavesError> {
    let panel = panels
        .iter()
        .find(|panel| panel.set_id == set)
        .ok_or_else(|| {
            DoorLeavesError::Unreadable(format!("panel set #{} is not the door's", set.0))
        })?;
    // Upstream reads `PanelDepth` as a positive length; zero is no depth.
    Ok(length(model, panel, "PanelDepth")?.filter(|depth| *depth > 0.0))
}

fn leaf(leaf: &Leaf, depth: Option<f64>) -> Result<DoorLeaf, DoorLeavesError> {
    let [x, y, z] = leaf.frame_world.basis;
    let (along, opening, up) = (direction(x)?, direction(y)?, direction(z)?);
    let position = match leaf.position {
        PanelPosition::Left => LeafPosition::Left,
        PanelPosition::Middle => LeafPosition::Middle,
        PanelPosition::Right => LeafPosition::Right,
        PanelPosition::NotDefined => LeafPosition::NotDefined,
    };
    let motion = match leaf.motion {
        openbim_ifc::LeafMotion::Swing => LeafMotion::Swing,
        openbim_ifc::LeafMotion::DoubleSwing => LeafMotion::DoubleSwing,
        openbim_ifc::LeafMotion::Slide { direction: slide } => LeafMotion::Slide(direction(slide)?),
        openbim_ifc::LeafMotion::RollUp => LeafMotion::RollUp,
        openbim_ifc::LeafMotion::Fixed => LeafMotion::Fixed,
        other => {
            return Err(DoorLeavesError::Refused(format!(
                "leaf motion {other:?} is not mapped"
            )));
        }
    };
    let hinge_side = leaf.hinge_side.map(|side| match side {
        Side::Left => HingeSide::Left,
        Side::Right => HingeSide::Right,
    });
    let swing = match &leaf.swing {
        None => None,
        Some(sector) => {
            let double = matches!(motion, LeafMotion::DoubleSwing);
            // Upstream starts a double-acting sweep at the leaf open to
            // local -y; the closed leaf lies a quarter turn on.
            let closed = if double {
                sector.direction_at(std::f64::consts::FRAC_PI_2)
            } else {
                sector.start
            };
            Some(SwingSector::try_new(
                sector.center,
                sector.radius,
                direction(closed)?,
                direction(sector.end())?,
                double,
            )?)
        }
    };
    DoorLeaf::try_new(
        position,
        motion,
        leaf.frame_world.origin,
        along,
        opening,
        up,
        leaf.width,
        depth,
        hinge_side,
        swing,
    )
}

/// The leaves of `door` (entity `id`), cited under `fingerprint`: a door's
/// leaves, or a window's panels (`crate::windows`) when it is a window.
pub(crate) fn door_leaves(
    model: &Model,
    fingerprint: &str,
    door: &ObjectId,
    id: EntityId,
) -> Result<DoorLeaves, DoorLeavesError> {
    let operation: DoorOperation = match door_operation(model, id) {
        Ok(operation) => operation,
        Err(DoorOperationError::NotADoor { .. }) => {
            return crate::windows::window_leaves(model, fingerprint, door, id);
        }
        Err(error) => return Err(operation_error(door, &error)),
    };
    let name = operation_name(operation.operation)?;
    let (lining_thickness, lining_set) = lining(model, id, "IfcDoorLiningProperties")?;
    let panels = exact_predefined_sets(model, id, "IfcDoorPanelProperties")
        .map_err(|error| property_error(&error))?;
    let leaves = operation
        .leaves
        .iter()
        .map(|placed| leaf(placed, panel_depth(model, &panels, placed.panel_set)?))
        .collect::<Result<Vec<_>, _>>()?;
    let sets = operation
        .leaves
        .iter()
        .map(|leaf| format!("#{}", leaf.panel_set.0))
        .collect::<Vec<_>>()
        .join(",");
    let lining = lining_set.map_or(String::new(), |set| format!(":lining=#{}", set.0));
    let evidence = Evidence::exact(
        door.source.clone(),
        format!("ifc:{fingerprint}:door-operation:{id}:{name}:panels={sets}{lining}"),
    );
    DoorLeaves::try_new(
        door.clone(),
        name,
        operation.overall_width,
        lining_thickness,
        leaves,
        evidence,
    )
}
