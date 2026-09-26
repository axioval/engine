//! Presentation layers an object's shape is assigned to.
//!
//! IFC assigns layers with `IfcPresentationLayerAssignment`, whose
//! `AssignedItems` name shape representations or their items, not products.
//! An object is on a layer when its own shape representation, one of that
//! representation's items, or the representation an `IfcMappedItem` maps in
//! (a type's shared geometry) is assigned to it.

use std::collections::{BTreeMap, BTreeSet};

use ifc_model::{EntityId, Model, Value};
use ifc_schema::Schema;

/// Assigned item -> (layer name, assignment instance).
pub(crate) type LayerIndex = BTreeMap<EntityId, Vec<(String, EntityId)>>;

/// Every layer assignment in the model, keyed by the item it assigns.
pub(crate) fn index(schema: &Schema, model: &Model) -> Result<LayerIndex, String> {
    let names = schema.attribute_names("IfcPresentationLayerAssignment");
    let slot = |name: &str| {
        names
            .iter()
            .position(|candidate| *candidate == name)
            .ok_or_else(|| format!("IfcPresentationLayerAssignment declares no {name}"))
    };
    let (name_slot, items_slot) = (slot("Name")?, slot("AssignedItems")?);
    let mut types = vec!["IfcPresentationLayerAssignment"];
    types.extend(schema.subtypes("IfcPresentationLayerAssignment"));
    let mut index = LayerIndex::new();
    for type_name in types {
        for assignment in model.ids_of_type(type_name) {
            let malformed = |what: &str| format!("{assignment} ({type_name}) {what}");
            let entity = model
                .get(*assignment)
                .ok_or_else(|| malformed("is indexed but absent"))?;
            let name = match entity.attribute(name_slot).map(Value::unwrap_typed) {
                Some(Value::Text(name)) => name.to_string(),
                _ => return Err(malformed("has no layer name")),
            };
            let Some(Value::List(items)) = entity.attribute(items_slot) else {
                return Err(malformed("has no AssignedItems list"));
            };
            for item in items {
                let Value::Ref(item) = item else {
                    return Err(malformed("assigns something that is not a reference"));
                };
                index
                    .entry(*item)
                    .or_default()
                    .push((name.clone(), *assignment));
            }
        }
    }
    Ok(index)
}

/// The distinct layers of `object`, each with the assignment that states it.
pub(crate) fn layers_of(
    schema: &Schema,
    model: &Model,
    index: &LayerIndex,
    object: EntityId,
) -> Result<BTreeMap<String, EntityId>, String> {
    let mut found = BTreeMap::new();
    let Some(shape) = reference(schema, model, object, "Representation")? else {
        return Ok(found);
    };
    let mut seen = BTreeSet::new();
    for representation in references(schema, model, shape, "Representations")? {
        visit(
            schema,
            model,
            index,
            representation,
            &mut found,
            &mut seen,
            0,
        )?;
    }
    Ok(found)
}

/// A representation, its items, and what its mapped items map in.
fn visit(
    schema: &Schema,
    model: &Model,
    index: &LayerIndex,
    representation: EntityId,
    found: &mut BTreeMap<String, EntityId>,
    seen: &mut BTreeSet<EntityId>,
    depth: usize,
) -> Result<(), String> {
    const MAX_DEPTH: usize = 8;
    if !seen.insert(representation) {
        return Ok(());
    }
    if depth > MAX_DEPTH {
        return Err(format!(
            "{representation} maps geometry deeper than {MAX_DEPTH} levels"
        ));
    }
    record(index, representation, found);
    for item in references(schema, model, representation, "Items")? {
        record(index, item, found);
        let is_mapped = model
            .get(item)
            .is_some_and(|entity| schema.is_a(&entity.type_name, "IFCMAPPEDITEM"));
        if is_mapped {
            if let Some(map) = reference(schema, model, item, "MappingSource")? {
                if let Some(mapped) = reference(schema, model, map, "MappedRepresentation")? {
                    visit(schema, model, index, mapped, found, seen, depth + 1)?;
                }
            }
        }
    }
    Ok(())
}

fn record(index: &LayerIndex, id: EntityId, found: &mut BTreeMap<String, EntityId>) {
    for (name, assignment) in index.get(&id).into_iter().flatten() {
        found.entry(name.clone()).or_insert(*assignment);
    }
}

fn slot_value<'m>(
    schema: &Schema,
    model: &'m Model,
    id: EntityId,
    attribute: &str,
) -> Result<Option<&'m Value>, String> {
    let entity = model
        .get(id)
        .ok_or_else(|| format!("{id} is referenced but absent"))?;
    let Some(slot) = schema
        .attribute_names(&entity.type_name)
        .iter()
        .position(|name| *name == attribute)
    else {
        return Ok(None);
    };
    Ok(entity.attribute(slot))
}

fn reference(
    schema: &Schema,
    model: &Model,
    id: EntityId,
    attribute: &str,
) -> Result<Option<EntityId>, String> {
    match slot_value(schema, model, id, attribute)? {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Ref(target)) => Ok(Some(*target)),
        Some(_) => Err(format!("{id}.{attribute} is not a reference")),
    }
}

fn references(
    schema: &Schema,
    model: &Model,
    id: EntityId,
    attribute: &str,
) -> Result<Vec<EntityId>, String> {
    match slot_value(schema, model, id, attribute)? {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::List(values)) => values
            .iter()
            .map(|value| match value {
                Value::Ref(target) => Ok(*target),
                _ => Err(format!(
                    "{id}.{attribute} lists something that is not a reference"
                )),
            })
            .collect(),
        Some(_) => Err(format!("{id}.{attribute} is not a list")),
    }
}
