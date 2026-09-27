//! Door leaves as an object-frame service states them, for capabilities
//! that read door swings.

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    DoorLeaf, DoorLeaves, DoorLeavesError, HingeSide, LeafMotion, LeafPosition, MetricDirection,
    ObjectFrame, ObjectFrameError, ObjectFrameService, ObjectFrameServiceHandle, SourceSnapshot,
    SwingSector,
};
use axioval_ir::{Evidence, ObjectId};

use super::{id, source};

fn direction(vector: [f64; 3]) -> MetricDirection {
    MetricDirection::try_new(vector).unwrap()
}

/// Doors by local id: their leaves, or why they are unknown. Objects not
/// listed are not doors. No object is placed.
pub struct Doors {
    snapshots: Vec<SourceSnapshot>,
    leaves: BTreeMap<ObjectId, Result<DoorLeaves, DoorLeavesError>>,
}

impl Default for Doors {
    fn default() -> Self {
        Self {
            snapshots: vec![SourceSnapshot::try_new(source(), "r1", "sha256:1").unwrap()],
            leaves: BTreeMap::new(),
        }
    }
}

/// One hinged leaf `width` wide, hinged at `hinge`, closed towards
/// `closed` and opening towards `open` (both horizontal).
pub fn hinged(
    hinge: [f64; 3],
    closed: [f64; 3],
    open: [f64; 3],
    width: f64,
    double: bool,
) -> DoorLeaf {
    let side = if closed[0] * open[1] - closed[1] * open[0] > 0.0 {
        HingeSide::Left
    } else {
        HingeSide::Right
    };
    let sector =
        SwingSector::try_new(hinge, width, direction(closed), direction(open), double).unwrap();
    DoorLeaf::try_new(
        LeafPosition::NotDefined,
        if double {
            LeafMotion::DoubleSwing
        } else {
            LeafMotion::Swing
        },
        hinge,
        direction(closed),
        direction(open),
        direction([0.0, 0.0, 1.0]),
        width,
        Some(0.04),
        Some(side),
        Some(sector),
    )
    .unwrap()
}

/// One sliding leaf from `origin` along `along`.
pub fn sliding(origin: [f64; 3], along: [f64; 3], width: f64) -> DoorLeaf {
    let along = direction(along);
    let [x, y, _] = along.components();
    DoorLeaf::try_new(
        LeafPosition::NotDefined,
        LeafMotion::Slide(along),
        origin,
        along,
        direction([-y, x, 0.0]),
        direction([0.0, 0.0, 1.0]),
        width,
        None,
        None,
        None,
    )
    .unwrap()
}

impl Doors {
    /// `local` is a door with `leaves`, `overall` wide with a lining
    /// `lining` thick.
    pub fn door(
        mut self,
        local: &str,
        leaves: Vec<DoorLeaf>,
        overall: f64,
        lining: Option<f64>,
    ) -> Self {
        let door = id(local);
        let answer = DoorLeaves::try_new(
            door.clone(),
            "TEST",
            overall,
            lining,
            leaves,
            Evidence::exact(source(), format!("leaves:{local}")),
        )
        .unwrap();
        self.leaves.insert(door, Ok(answer));
        self
    }

    /// `local` is a door whose leaves cannot be read.
    pub fn unknown(mut self, local: &str, error: DoorLeavesError) -> Self {
        self.leaves.insert(id(local), Err(error));
        self
    }

    pub fn handle(self) -> ObjectFrameServiceHandle {
        ObjectFrameServiceHandle::new(Arc::new(self))
    }
}

impl ObjectFrameService for Doors {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn object_frame(&self, object: &ObjectId) -> Result<ObjectFrame, ObjectFrameError> {
        Err(ObjectFrameError::NotPlaced(object.clone()))
    }

    fn leaves(&self, door: &ObjectId) -> Result<DoorLeaves, DoorLeavesError> {
        self.leaves
            .get(door)
            .cloned()
            .unwrap_or_else(|| Err(DoorLeavesError::NotADoor(door.clone())))
    }
}
