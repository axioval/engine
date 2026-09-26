//! Property-set definitions exact resolution does not read.
//!
//! `ifc-properties::exact_property` resolves members of `IfcPropertySet` and
//! skips every other `IfcPropertySetDefinition`: quantity sets
//! (`IfcElementQuantity`) and predefined property sets such as
//! `IfcDoorLiningProperties`. Its `Absent` is therefore "in no property set",
//! which is narrower than the complete absence the engine's evidence claims:
//! a quantity named `Foo` in a set named `Foo_Bar` would be reported absent
//! from `Foo_Bar`.
//!
//! This index lets the property service refuse that claim. It records, once
//! per session, the name of every skipped definition and every name a
//! member of one could be looked up by: quantity names (nested complex
//! quantities included) and the attributes a predefined set declares. It is
//! model-wide, not per object, so it can only turn an absence into "not
//! evaluated", never hide a present value.

use std::collections::BTreeSet;

use ifc_model::{EntityId, Model, Value};

use crate::release::Release;

/// Names that an absence claim must not cover.
#[derive(Debug, Default)]
pub(crate) struct UnreadDefinitions {
    /// `Name` of every skipped definition.
    sets: BTreeSet<String>,
    /// Every member name a skipped definition could answer a lookup with.
    members: BTreeSet<String>,
}

impl UnreadDefinitions {
    pub(crate) fn read(release: Release, model: &Model) -> Self {
        let schema = release.schema;
        let inherited: BTreeSet<&str> = schema
            .attribute_names("IfcPropertySetDefinition")
            .into_iter()
            .collect();
        let mut unread = Self::default();
        for (_, entity) in model.iter() {
            let kind = entity.type_name.as_ref();
            if !schema.is_a(kind, "IFCPROPERTYSETDEFINITION") || schema.is_a(kind, "IFCPROPERTYSET")
            {
                continue;
            }
            let names = schema.attribute_names(kind);
            if let Some(name) =
                position(&names, "Name").and_then(|slot| text(entity.attribute(slot)))
            {
                unread.sets.insert(name);
            }
            if schema.is_a(kind, "IFCELEMENTQUANTITY") {
                let quantities =
                    position(&names, "Quantities").and_then(|slot| entity.attribute(slot));
                unread.quantities(release, model, quantities, &mut BTreeSet::new());
            } else {
                // A predefined set's members are its own attributes.
                unread.members.extend(
                    names
                        .iter()
                        .filter(|name| !inherited.contains(*name))
                        .map(|name| (*name).to_owned()),
                );
            }
        }
        unread
    }

    /// Records the names of `quantities`, descending into complex ones.
    fn quantities(
        &mut self,
        release: Release,
        model: &Model,
        quantities: Option<&Value>,
        seen: &mut BTreeSet<EntityId>,
    ) {
        let schema = release.schema;
        for id in references(quantities) {
            // A cycle is malformed, and exact resolution never reads it.
            if !seen.insert(id) {
                continue;
            }
            let Some(quantity) = model.get(id) else {
                continue;
            };
            let kind = quantity.type_name.as_ref();
            let names = schema.attribute_names(kind);
            let Some(name_slot) = position(&names, "Name") else {
                continue;
            };
            if let Some(name) = text(quantity.attribute(name_slot)) {
                self.members.insert(name);
            }
            if schema.is_a(kind, "IFCPHYSICALCOMPLEXQUANTITY") {
                if let Some(slot) = position(&names, "HasQuantities") {
                    self.quantities(release, model, quantity.attribute(slot), seen);
                }
            }
        }
    }

    /// Why an absence of `property` from `set` is not proven, if it is not.
    pub(crate) fn obscures(&self, set: Option<&str>, property: &str) -> Option<String> {
        match set {
            Some(set) if self.sets.contains(set) => Some(format!(
                "`{set}` is a quantity or predefined property set, which exact resolution does not read"
            )),
            None if self.members.contains(property) => Some(format!(
                "`{property}` names a member of a quantity or predefined property set, which exact resolution does not read"
            )),
            _ => None,
        }
    }
}

fn position(names: &[&str], wanted: &str) -> Option<usize> {
    names.iter().position(|name| *name == wanted)
}

fn text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Text(text) => Some(text.to_string()),
        _ => None,
    }
}

fn references(value: Option<&Value>) -> Vec<EntityId> {
    match value {
        Some(Value::List(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::Ref(id) => Some(*id),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}
