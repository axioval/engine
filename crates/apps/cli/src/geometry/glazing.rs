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

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use axioval::axiolid::{GlazingItem, GlazingItems, ItemMaterial};
use axioval::ifc::{GlazingReader, ItemGlazing};
use axioval::ir::{ObjectId, SourceId};
use ifc_geometry::lower::lower_representation_item;

use super::{Fit, NO_TRIANGLES, Parsed, compile, entity_id, session};

/// A door's items, or why they cannot be read; `None` for no body.
type Items = Result<Option<Arc<Vec<GlazingItem>>>, String>;

/// Every source's body items, read door by door on request.
pub(super) struct BodyItems {
    parsed: Arc<BTreeMap<SourceId, Parsed>>,
    readers: BTreeMap<SourceId, OnceLock<Result<GlazingReader, String>>>,
    items: Mutex<BTreeMap<ObjectId, Items>>,
}

impl BodyItems {
    pub(super) fn new(parsed: Arc<BTreeMap<SourceId, Parsed>>) -> Self {
        let readers = parsed
            .keys()
            .map(|source| (source.clone(), OnceLock::new()))
            .collect();
        Self {
            parsed,
            readers,
            items: Mutex::new(BTreeMap::new()),
        }
    }

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
        if let Some(items) = self
            .items
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(door)
        {
            return items.clone();
        }
        let items = self.read(door);
        self.items
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(door.clone(), items.clone());
        items
    }
}
