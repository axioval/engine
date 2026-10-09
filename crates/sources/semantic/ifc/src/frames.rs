//! Object frames from IFC object placements.
//!
//! An `IfcProduct` is placed by its `ObjectPlacement`: a chain of
//! `IfcLocalPlacement`s, each an `IfcAxis2Placement` relative to its parent.
//! The chain is resolved by `ifc-geometry`'s `PlacementResolver`, the same
//! composition that places the product's body, so a frame and the geometry
//! meshed from the file agree. Missing `Axis` and `RefDirection` take the
//! schema's defaults there, and a non-perpendicular `RefDirection` is
//! projected as `IfcBuildAxes` prescribes.
//!
//! The composed origin is in the project length unit and is converted to
//! metres through `ifc_properties::exact_unit`, which refuses rather than
//! assuming metres. The axes are dimensionless.
//!
//! IFC does not state a product's front: the placement's Y axis is an
//! authoring convention, not a statement of which side a component is used
//! from. Every frame from this service therefore reports
//! `ObjectFront::NotStated`.
//!
//! The same service answers a door's leaves (`crate::doors`) and a window's
//! panels (`crate::windows`), cached per object.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axioval_engine::{
    DoorLeaves, DoorLeavesError, MetricDirection, MetricFrame, MetricPoint, ObjectFrame,
    ObjectFrameError, ObjectFrameService, ObjectFront, SourceSnapshot,
};
use axioval_ir::{Evidence, ObjectId};
use ifc_geometry::GeometryError;
use ifc_geometry::constraint::local::{LocalPlacement, PlacementResolver};
use ifc_model::{EntityId, Model, Value};
use ifc_properties::exact_unit;

use crate::release::Release;

pub(crate) struct IfcObjectFrames {
    release: Release,
    model: Arc<Model>,
    snapshots: Arc<[SourceSnapshot]>,
    /// Metres per project length unit, or why it cannot be resolved exactly.
    metres_per_unit: Result<f64, String>,
    /// Placement chains shared between products (storey, building, site).
    resolver: Mutex<PlacementResolver>,
    /// Door leaves already derived: each derivation validates the file's
    /// property relationships.
    doors: Mutex<BTreeMap<EntityId, Result<DoorLeaves, DoorLeavesError>>>,
}

impl IfcObjectFrames {
    pub(crate) fn new(
        release: Release,
        model: Arc<Model>,
        snapshots: Arc<[SourceSnapshot]>,
    ) -> Self {
        let metres_per_unit = match exact_unit(&model, "IFCLENGTHMEASURE", None) {
            Ok(unit) if unit.offset == 0.0 && unit.scale.is_finite() && unit.scale > 0.0 => {
                Ok(unit.scale)
            }
            Ok(_) => Err("the project length unit has no positive finite scale".into()),
            Err(error) => Err(format!(
                "the project length unit cannot be resolved exactly: {error}"
            )),
        };
        Self {
            release,
            model,
            snapshots,
            metres_per_unit,
            resolver: Mutex::new(PlacementResolver::new()),
            doors: Mutex::new(BTreeMap::new()),
        }
    }

    fn entity(&self, object: &ObjectId) -> Result<EntityId, ObjectFrameError> {
        let id = object
            .local_id
            .strip_prefix('#')
            .and_then(|digits| digits.parse::<u64>().ok())
            .map(EntityId)
            .ok_or_else(|| ObjectFrameError::UnknownObject(object.clone()))?;
        if self.model.get(id).is_none() {
            return Err(ObjectFrameError::UnknownObject(object.clone()));
        }
        Ok(id)
    }

    /// The object's `ObjectPlacement`, by its slot in this release's schema.
    fn placement(&self, object: &ObjectId, id: EntityId) -> Result<EntityId, ObjectFrameError> {
        let entity = self
            .model
            .get(id)
            .ok_or_else(|| ObjectFrameError::UnknownObject(object.clone()))?;
        let schema = self.release.schema;
        if !schema.is_a(&entity.type_name, "IFCPRODUCT") {
            // Only products carry a placement; a group or a process has none.
            return Err(ObjectFrameError::NotPlaced(object.clone()));
        }
        let slot = schema
            .attribute_names(&entity.type_name)
            .iter()
            .position(|name| name.eq_ignore_ascii_case("ObjectPlacement"))
            .ok_or_else(|| {
                ObjectFrameError::Unreadable(format!(
                    "{} declares no ObjectPlacement",
                    entity.type_name
                ))
            })?;
        match entity.attribute(slot) {
            Some(Value::Ref(placement)) => Ok(*placement),
            Some(Value::Null) => Err(ObjectFrameError::NotPlaced(object.clone())),
            _ => Err(ObjectFrameError::Unreadable(format!(
                "{id}.ObjectPlacement is not a reference"
            ))),
        }
    }

    /// The first placement above the local placement `placement` that is
    /// no `IfcLocalPlacement`, with its entity type. A missing parent and a
    /// cycle are left to the resolver, which refuses them.
    fn non_local_ancestor(&self, placement: EntityId) -> Option<(EntityId, String)> {
        let mut seen = vec![placement];
        let mut current = placement;
        loop {
            let parent = LocalPlacement::new(current, self.model.get(current)?).parent()?;
            if seen.contains(&parent) {
                return None;
            }
            let kind = self.model.get(parent)?.type_name.to_ascii_uppercase();
            if kind != "IFCLOCALPLACEMENT" {
                return Some((parent, kind));
            }
            seen.push(parent);
            current = parent;
        }
    }

    /// The placement chain from `placement` up to its root, as STEP ids.
    ///
    /// Called after the resolver accepted the chain, so it is acyclic and
    /// made of local placements only.
    fn chain(&self, placement: EntityId) -> Vec<EntityId> {
        let mut chain = Vec::new();
        let mut current = Some(placement);
        while let Some(id) = current {
            if chain.contains(&id) {
                break;
            }
            chain.push(id);
            current = self
                .model
                .get(id)
                .and_then(|entity| LocalPlacement::new(id, entity).parent());
        }
        chain
    }
}

fn geometry_error(error: GeometryError) -> ObjectFrameError {
    match error {
        GeometryError::Unsupported { .. } => ObjectFrameError::Unsupported(error.to_string()),
        other => ObjectFrameError::Unreadable(other.to_string()),
    }
}

impl ObjectFrameService for IfcObjectFrames {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn object_frame(&self, object: &ObjectId) -> Result<ObjectFrame, ObjectFrameError> {
        let id = self.entity(object)?;
        let placement = self.placement(object, id)?;
        let kind = self
            .model
            .get(placement)
            .map(|entity| entity.type_name.to_ascii_uppercase())
            .ok_or_else(|| {
                ObjectFrameError::Unreadable(format!("{id}.ObjectPlacement {placement} is missing"))
            })?;
        if kind != "IFCLOCALPLACEMENT" {
            // `IfcGridPlacement` places by grid intersections; nothing here
            // resolves it exactly, so it is refused rather than approximated.
            return Err(ObjectFrameError::Unsupported(format!(
                "{id} is placed by {kind} {placement}; only IfcLocalPlacement chains are resolved"
            )));
        }
        // `ifc-geometry` 0.11 resolves a local placement relative to a grid
        // or linear one too; this service still refuses such a chain, as it
        // refuses the grid placement itself.
        if let Some((parent, kind)) = self.non_local_ancestor(placement) {
            return Err(ObjectFrameError::Unsupported(format!(
                "{id} is placed relative to {kind} {parent}; only IfcLocalPlacement chains are \
                 resolved"
            )));
        }
        let metres = self
            .metres_per_unit
            .clone()
            .map_err(ObjectFrameError::Unreadable)?;
        let transform = self
            .resolver
            .lock()
            .map_err(|_| ObjectFrameError::Unreadable("placement cache lock is poisoned".into()))?
            .world_transform(&self.model, placement)
            .map_err(geometry_error)?;

        let invalid = |_| ObjectFrameError::InvalidFrame;
        let origin = MetricPoint::try_new(object.clone(), transform.origin.map(|c| c * metres))
            .map_err(invalid)?;
        let [x, y, z] = transform.basis;
        let frame = MetricFrame::try_new(
            origin,
            MetricDirection::try_new(x).map_err(|_| ObjectFrameError::InvalidFrame)?,
            MetricDirection::try_new(y).map_err(|_| ObjectFrameError::InvalidFrame)?,
            MetricDirection::try_new(z).map_err(|_| ObjectFrameError::InvalidFrame)?,
        )
        .map_err(|_| ObjectFrameError::InvalidFrame)?;

        let chain = self
            .chain(placement)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("<");
        let evidence = Evidence::exact(
            object.source.clone(),
            format!(
                "ifc:{}:placement:{id}:{chain}",
                self.snapshots[0].fingerprint()
            ),
        );
        ObjectFrame::try_new(object.clone(), frame, ObjectFront::NotStated, evidence)
    }

    fn leaves(&self, door: &ObjectId) -> Result<DoorLeaves, DoorLeavesError> {
        let id = self.entity(door).map_err(|error| match error {
            ObjectFrameError::UnknownObject(object) => DoorLeavesError::UnknownObject(object),
            other => DoorLeavesError::Unreadable(other.to_string()),
        })?;
        let mut cache = self
            .doors
            .lock()
            .map_err(|_| DoorLeavesError::Unreadable("door cache lock is poisoned".into()))?;
        cache
            .entry(id)
            .or_insert_with(|| {
                crate::doors::door_leaves(&self.model, self.snapshots[0].fingerprint(), door, id)
            })
            .clone()
    }
}
