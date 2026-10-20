//! The items of a door's body, for the glazing of its leaves (#205).
//!
//! The leaf-glazing service measures each leaf's glazed area from the
//! door's body item by item. Its items are read here, on the first request
//! for a door and once: `ifc-geometry`'s `body_description` lists the
//! body's geometric items (mapped items resolved, each with its world frame
//! and the mapped items it was reached through), each is lowered and
//! compiled alone as the bodies are (`compile`), and the IFC adapter's
//! `GlazingReader` says from its styles and materials whether it is
//! glazing. A body the description refuses, or an item that cannot be
//! meshed, refuses the door's items whole: a leaf is never measured from
//! part of its body. An item that compiles to no triangles (a curve or
//! point in the body) occupies nothing and is left out. Openings voiding
//! the door are not subtracted; doors are not voided in practice, and the
//! glazing of a leaf is read from its items as authored.
//!
//! A door's items are a derivation of the check's store ([`DoorItems`],
//! ADR 0006, #443): read once at a time for every leaf, rule and thread
//! asking, counted against the check's memory budget and released after
//! the last rule that may ask the leaf-glazing service. Each source's
//! glazing reader (its styles and materials, indexed) stays with the
//! service: an index of the model, read once.

use std::any::TypeId;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use axioval_axiolid::{GlazingItem, GlazingItems, ItemMaterial};
use axioval_engine::{
    DerivationError, DerivationKind, DerivationStore, DerivationVersion, Inputs, Kernel,
    LeafGlazingServiceHandle, Precision,
};
use axioval_ifc::{GlazingReader, ItemGlazing};
use axioval_ir::{ObjectId, SourceId};
use ifc_geometry::lower::lower_representation_item;

use super::{Fit, NO_TRIANGLES, Parsed, compile, entity_id, session};

/// A door's items, or why they cannot be read; `None` for no body.
type Items = Result<Option<Arc<Vec<GlazingItem>>>, String>;

/// Every source's body items, read door by door on request: the check's,
/// read once per door ([`DoorItems`]).
pub(super) struct BodyItems {
    readers: Arc<Readers>,
}

/// The models a door's items are read from, with each source's glazing
/// reader, indexed once on its first door: the model's styles and
/// materials, part of the source as its parse is.
pub(super) struct Readers {
    parsed: Arc<BTreeMap<SourceId, Parsed>>,
    readers: BTreeMap<SourceId, OnceLock<Result<GlazingReader, String>>>,
}

impl BodyItems {
    pub(super) fn new(parsed: Arc<BTreeMap<SourceId, Parsed>>) -> Self {
        let readers = parsed
            .keys()
            .map(|source| (source.clone(), OnceLock::new()))
            .collect();
        Self {
            readers: Arc::new(Readers { parsed, readers }),
        }
    }
}

impl Readers {
    fn read(&self, door: &ObjectId) -> Items {
        let parsed = self
            .parsed
            .get(&door.source)
            .ok_or_else(|| format!("no model for source `{}`", door.source))?;
        let reader = self
            .readers
            .get(&door.source)
            .ok_or_else(|| format!("no model for source `{}`", door.source))?
            .get_or_init(|| GlazingReader::new(&parsed.model))
            .as_ref()
            .map_err(|why| format!("the model's styles cannot be read: {why}"))?;
        let entity = entity_id(door).ok_or_else(|| "not a STEP instance id".to_owned())?;
        if let Some(reason) = parsed.linear.refusal(&parsed.model, entity) {
            return Err(reason);
        }
        let Some(description) =
            ifc_geometry::body_description(&parsed.model, &parsed.units, entity)
                .map_err(|error| format!("the body cannot be described: {error}"))?
        else {
            return Ok(None);
        };
        let backend = ifc_geometry::compile::default_backend();
        let mut items = Vec::with_capacity(description.items.len());
        for item in &description.items {
            let label = format!("#{}", item.item.0);
            let mut lowering = session(&parsed.model, &parsed.units);
            let node = lower_representation_item(&mut lowering, item.item, item.item_world)
                .map_err(|error| format!("item {label} cannot be lowered: {error}"))?;
            let lowered = lowering
                .finish(node)
                .map_err(|error| format!("item {label} cannot be lowered: {error}"))?;
            let (mesh, fit) = match compile(&backend, &lowered.graph, lowered.root) {
                Ok(compiled) => compiled,
                Err(reason) if reason == NO_TRIANGLES => continue,
                Err(reason) => return Err(format!("item {label} cannot be meshed: {reason}")),
            };
            let material =
                match reader.item_glazing(&parsed.model, entity, &item.mapped_by, item.item) {
                    ItemGlazing::Glazed => ItemMaterial::Glazed,
                    ItemGlazing::Opaque => ItemMaterial::Opaque,
                    ItemGlazing::Undecided(why) => ItemMaterial::Undecided(why),
                };
            items.push(GlazingItem {
                label,
                mesh,
                deviation: match fit {
                    Fit::Exact => 0.0,
                    Fit::Within(deviation) => deviation,
                },
                material,
            });
        }
        if items.is_empty() {
            return Ok(None);
        }
        Ok(Some(Arc::new(items)))
    }
}

impl GlazingItems for BodyItems {
    fn items(&self, door: &ObjectId) -> Items {
        let key = ItemsKey {
            readers: Over(Arc::clone(&self.readers)),
            door: door.clone(),
            precision: Precision::Exact,
        };
        DerivationStore::derive_current::<DoorItems>(&key)
            .map_err(|error| error.to_string())?
            .as_ref()
            .clone()
    }
}

/// The models read, compared and hashed by their address: two keys over
/// one service's models are equal in this field, keys over two services
/// never.
#[derive(Clone)]
pub(super) struct Over(Arc<Readers>);

impl PartialEq for Over {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Over {}

impl Hash for Over {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

/// Everything a door's items are read from (#439): the models (by
/// identity), the door and the precision class (#438); the kind's version
/// beside it. The items are meshed as the bodies are, each with the
/// deviation it is meshed within as data, and no fast mode runs: every
/// reader asks Exact.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct ItemsKey {
    readers: Over,
    door: ObjectId,
    precision: Precision,
}

/// The kernel crates a door's items are read and meshed with, at the
/// versions the workspace locks.
const IFC_GEOMETRY: Kernel = Kernel {
    name: "ifc-geometry",
    version: "0.29.0",
};
const AXIOLID_MESH_COMPILE: Kernel = Kernel {
    name: "axiolid-mesh-compile",
    version: "0.3.29",
};

/// The derivation kind of doors' body items.
pub(super) struct DoorItems;

impl DerivationKind for DoorItems {
    const NAME: &'static str = "door items";
    /// Bumped whenever the same door could read other items: the body
    /// description, the lowering, the meshing or the glazing reader
    /// changing what they return.
    const VERSION: DerivationVersion =
        DerivationVersion::new(1, &[IFC_GEOMETRY, AXIOLID_MESH_COMPILE]);
    type Key = ItemsKey;
    /// A door whose items cannot be read is refused alike for every leaf.
    type Value = Items;

    fn build(key: &ItemsKey, _: &Inputs<'_>) -> Result<Self::Value, DerivationError> {
        Ok(key.readers.0.read(&key.door))
    }

    fn precision(key: &ItemsKey) -> Option<Precision> {
        Some(key.precision)
    }

    fn size_hint(value: &Self::Value) -> Option<usize> {
        let item = |item: &GlazingItem| {
            item.mesh.positions.len() * std::mem::size_of::<axiolid_core::Point3>()
                + item.mesh.indices.len() * std::mem::size_of::<u32>()
                + item.label.len()
                + std::mem::size_of::<GlazingItem>()
        };
        Some(match value {
            Ok(Some(items)) => items.iter().map(item).sum(),
            _ => 0,
        })
    }

    /// Items are read by the leaf-glazing service alone.
    fn read_through() -> Option<Vec<TypeId>> {
        Some(vec![TypeId::of::<LeafGlazingServiceHandle>()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_engine::{KeyCompleteness, SourceSnapshot};
    use axioval_ifc::import_ifc_session;

    /// A door `#40` and a slab `#70`, each with a box body; `height` sets
    /// the door's.
    fn model(height: f64) -> String {
        let body = |first: u32, x: f64, dx: f64, depth: f64| {
            let [point, position, profile, solid, shape] = [0, 1, 2, 3, 4].map(|o| first + o);
            format!(
                "#{point}=IFCCARTESIANPOINT(({x},0.));\n\
                 #{position}=IFCAXIS2PLACEMENT2D(#{point},$);\n\
                 #{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#{position},{dx},0.1);\n\
                 #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,{depth});\n\
                 #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
                 #{}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n",
                first + 5
            )
        };
        format!(
            "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
             #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
             #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
             #3=IFCLOCALPLACEMENT($,#2);\n\
             #4=IFCDIRECTION((0.,0.,1.));\n\
             #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
             {}#40=IFCDOOR('0000000000000000000040',$,$,$,$,#3,#35,$,{height},0.9,$,$,$);\n\
             {}#70=IFCSLAB('0000000000000000000070',$,$,$,$,#3,#65,$,.FLOOR.);\n\
             ENDSEC;\nEND-ISO-10303-21;\n",
            body(30, 0.0, 0.9, height),
            body(60, 2.0, 4.0, 0.2),
        )
    }

    /// The items of one parse of `model(height)`, and the source.
    fn items(height: f64) -> (BodyItems, SourceId) {
        let bytes = model(height);
        let session = import_ifc_session("door.ifc", bytes.as_bytes()).unwrap();
        let snapshots: Vec<SourceSnapshot> = session.snapshots().cloned().collect();
        let models: super::super::ModelBytes = snapshots
            .iter()
            .map(|snapshot| (snapshot.source().clone(), bytes.as_bytes().to_vec()))
            .collect();
        let parsed = super::super::parse(&snapshots, &models).unwrap();
        let source = snapshots[0].source().clone();
        (BodyItems::new(Arc::new(parsed)), source)
    }

    fn read(value: &Items) -> String {
        match value {
            Ok(Some(items)) => format!(
                "{:?}",
                items
                    .iter()
                    .map(|item| (&item.label, item.mesh.positions.clone(), item.deviation))
                    .collect::<Vec<_>>()
            ),
            Ok(None) => "no body".to_owned(),
            Err(reason) => format!("refused: {reason}"),
        }
    }

    /// Every input a door's items are read from is in their key (#439).
    #[test]
    fn changing_any_input_of_a_door_s_items_changes_their_key() {
        let ((one, source), (other, _)) = (items(2.1), items(2.0));
        let key = |items: &BodyItems, local: &str| ItemsKey {
            readers: Over(Arc::clone(&items.readers)),
            door: ObjectId {
                source: source.clone(),
                local_id: local.to_owned(),
            },
            precision: Precision::Exact,
        };
        let base = key(&one, "#40");
        KeyCompleteness::<DoorItems, _>::new(base.clone(), read)
            .same("the same inputs", key(&one, "#40"))
            .changes_value("the models", key(&other, "#40"))
            .changes_value("the door", key(&one, "#70"))
            .precision(
                "the precision class",
                ItemsKey {
                    precision: Precision::fast(1.0e-3),
                    ..base
                },
            )
            .check();
    }
}
