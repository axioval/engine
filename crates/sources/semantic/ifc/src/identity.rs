//! IFC GlobalIds as external object identities.
//!
//! An object's source-qualified id is its STEP instance number, which is
//! unique in the file but renumbered by every export. `IfcRoot.GlobalId` is the
//! identity other software uses across exports (issue exchange, model
//! comparison), so it is attached as an alias under [`IFC_GLOBAL_ID`].
//!
//! The alias is attached only when it is trustworthy. A GlobalId that is
//! malformed, or that another `IfcRoot` instance in the same file also claims,
//! is left off every object that carries it and reported through source
//! integrity instead. Attaching it anyway would let a consumer resolve the id
//! to the wrong object without noticing.
//!
//! The scan covers every `IfcRoot` instance, not only the objects the session
//! exposes: a relationship or type object sharing an element's GlobalId makes
//! that id just as ambiguous to a viewer resolving it.

use std::collections::BTreeMap;

use ifc_model::guid::Guid;
use ifc_model::{EntityId, Model, Value};

use crate::release::Release;

/// External id scheme of an IFC GlobalId (`IfcRoot.GlobalId`, 22 characters).
pub const IFC_GLOBAL_ID: &str = "ifc-globalid";

/// Why an instance's GlobalId was not attached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GlobalIdDefect {
    /// Not set, not text, or not a valid 22-character GlobalId.
    Invalid { instance: EntityId, found: String },
    /// Claimed by more than one `IfcRoot` instance, listed in file order.
    Duplicate {
        global_id: String,
        instances: Vec<EntityId>,
    },
}

/// Every `IfcRoot` GlobalId in one model, split into usable and defective.
#[derive(Debug, Default)]
pub(crate) struct GlobalIds {
    usable: BTreeMap<EntityId, String>,
    pub(crate) defects: Vec<GlobalIdDefect>,
}

impl GlobalIds {
    pub(crate) fn read(release: Release, model: &Model) -> Self {
        let schema = release.schema;
        let Some(slot) = schema
            .attribute_names("IfcRoot")
            .iter()
            .position(|name| *name == "GlobalId")
        else {
            // Both supported releases declare it; a table without it gets no aliases.
            return Self::default();
        };
        let mut claims: BTreeMap<String, Vec<EntityId>> = BTreeMap::new();
        let mut defects = Vec::new();
        for (id, entity) in model.iter() {
            if !schema.is_a(&entity.type_name, "IFCROOT") {
                continue;
            }
            match entity.attribute(slot).and_then(valid_global_id) {
                Some(global_id) => claims.entry(global_id).or_default().push(id),
                None => defects.push(GlobalIdDefect::Invalid {
                    instance: id,
                    found: entity
                        .attribute(slot)
                        .map_or_else(|| "no attribute".to_owned(), describe),
                }),
            }
        }
        let mut usable = BTreeMap::new();
        for (global_id, instances) in claims {
            if let [instance] = instances[..] {
                usable.insert(instance, global_id);
            } else {
                defects.push(GlobalIdDefect::Duplicate {
                    global_id,
                    instances,
                });
            }
        }
        Self { usable, defects }
    }

    /// The GlobalId of `instance`, when it is valid and unique in the file.
    pub(crate) fn of(&self, instance: EntityId) -> Option<&str> {
        self.usable.get(&instance).map(String::as_str)
    }
}

/// The text of a valid GlobalId.
///
/// `Guid::parse` checks only length and alphabet; it also accepts a leading
/// digit above `3`, which `to_uuid` silently truncates, so two ids could name
/// one UUID (openbimrs/ifc#62). The round trip rejects exactly those.
fn valid_global_id(value: &Value) -> Option<String> {
    let Value::Text(text) = value else {
        return None;
    };
    let guid = Guid::parse(text)?;
    (Guid::from_uuid(guid.to_uuid()) == guid).then(|| text.to_string())
}

fn describe(value: &Value) -> String {
    match value {
        Value::Null => "$".to_owned(),
        Value::Derived => "*".to_owned(),
        Value::Text(text) => format!("'{text}'"),
        other => format!("{other:?}"),
    }
}
