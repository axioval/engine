//! Direct IFC attributes (`Name`, `Tag`, `PredefinedType`, ...) of an object.
//!
//! An attribute is read from the object's own instance, in the one release
//! the file declares, by the name that release's schema gives its slot.
//! Nothing is inherited from a type object: an occurrence's `Name` is the
//! occurrence's.
//!
//! What a slot holds is mapped without guessing:
//!
//! - `$`, an empty aggregate, and a logical `.U.` are unset;
//! - text, enumeration items, booleans and integers are scalars carrying the
//!   declared type (a `SELECT` value carries the type it was written with);
//! - reals are scalars only when their type needs no unit (`IfcReal`,
//!   dimensionless `NUMBER` types); a measure is refused until unit context
//!   is read, since its number means nothing without it;
//! - references and non-empty aggregates are present but structured;
//! - derived (`*`) and binary values are refused.

use std::sync::Arc;

use std::collections::BTreeMap;

use axioval_engine::{
    AttributeError, AttributeService, AttributeValue, ResolvedAttribute, ResolvedPredefinedType,
    SourceSnapshot,
};
use axioval_ir::{Evidence, ObjectId, PropertyValue};
use ifc_model::{EntityId, Model, Value};

use crate::release::Release;

pub(crate) struct IfcAttributeService {
    release: Release,
    model: Arc<Model>,
    snapshots: Arc<[SourceSnapshot]>,
    /// Type objects by occurrence, from every `IfcRelDefinesByType`.
    types: BTreeMap<EntityId, Vec<EntityId>>,
}

impl IfcAttributeService {
    pub(crate) fn new(
        release: Release,
        model: Arc<Model>,
        snapshots: Arc<[SourceSnapshot]>,
    ) -> Self {
        let types = type_index(release, &model);
        Self {
            release,
            model,
            snapshots,
            types,
        }
    }

    fn entity_id(object: &ObjectId) -> Result<EntityId, AttributeError> {
        object
            .local_id
            .strip_prefix('#')
            .and_then(|digits| digits.parse::<u64>().ok())
            .map(EntityId)
            .ok_or_else(|| AttributeError::UnknownObject(object.clone()))
    }

    /// The text of an enumeration or string attribute: `Err` when the class
    /// has no such attribute, `Ok(None)` when it is unset.
    fn designation(&self, id: EntityId, name: &str) -> Result<Option<String>, ()> {
        let entity = self.model.get(id).ok_or(())?;
        let slot = self
            .release
            .schema
            .attribute_names(&entity.type_name)
            .iter()
            .position(|attribute| *attribute == name)
            .ok_or(())?;
        Ok(match entity.attribute(slot) {
            Some(Value::Text(text) | Value::Enum(text)) => Some(text.to_string()),
            _ => None,
        })
    }

    fn scalar(&self, value: &Value, declared: &str) -> Result<AttributeValue, AttributeError> {
        let data_type = Some(declared.to_ascii_uppercase()).filter(|name| !name.is_empty());
        let scalar = |value| {
            Ok(AttributeValue::Scalar {
                value,
                data_type: data_type.clone(),
            })
        };
        match value {
            Value::Null | Value::LogicalUnknown => Ok(AttributeValue::Unset),
            Value::List(items) if items.is_empty() => Ok(AttributeValue::Unset),
            Value::List(_) | Value::Ref(_) => Ok(AttributeValue::Structured),
            Value::Derived => Err(AttributeError::Unsupported(
                "a derived attribute has no stated value".into(),
            )),
            Value::Binary(_) => Err(AttributeError::Unsupported(
                "binary values are not represented".into(),
            )),
            Value::Integer(_) | Value::Real(_) if self.needs_unit(declared) => Err(unit(declared)),
            Value::Bool(value) => scalar(PropertyValue::Boolean(*value)),
            Value::Integer(value) => scalar(PropertyValue::Integer(*value)),
            Value::Text(text) => scalar(PropertyValue::String(text.to_string())),
            Value::Enum(item) => scalar(PropertyValue::String(item.to_string())),
            Value::Real(real) => scalar(PropertyValue::Decimal(*real)),
            // A SELECT slot names the member type it was written with.
            Value::Typed { type_name, value } => self.scalar(value, type_name),
        }
    }

    /// Whether a number of this declared type means nothing without a unit:
    /// a `REAL`-based defined type other than `IfcReal` itself.
    fn needs_unit(&self, declared: &str) -> bool {
        self.base(declared) == "REAL"
            && !declared.eq_ignore_ascii_case("IFCREAL")
            && !declared.eq_ignore_ascii_case("REAL")
    }

    fn base(&self, declared: &str) -> String {
        let base = self
            .release
            .schema
            .resolve_defined(declared)
            .to_ascii_uppercase();
        base.split('(').next().unwrap_or_default().trim().to_owned()
    }

    fn locator(&self, detail: impl std::fmt::Display) -> String {
        format!("ifc:{}:{detail}", self.snapshots[0].fingerprint())
    }
}

impl AttributeService for IfcAttributeService {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    /// Resolved as IDS reads it: the type object's designation first (its
    /// `PredefinedType`, or its `ElementType`/`ProcessType` when that is
    /// user-defined or unset) unless that is `NOTDEFINED` or empty, then the
    /// occurrence's (its `PredefinedType`, or its `ObjectType` when that is
    /// user-defined or unset).
    fn predefined_type(&self, object: &ObjectId) -> Result<ResolvedPredefinedType, AttributeError> {
        let id = Self::entity_id(object)?;
        if self.model.get(id).is_none() {
            return Err(AttributeError::UnknownObject(object.clone()));
        }
        let own = |name| self.designation(id, name).ok().flatten();
        let type_object = match self.types.get(&id).map(Vec::as_slice) {
            None | Some([]) => None,
            Some([single]) => Some(*single),
            Some(_) => {
                return Err(AttributeError::Unreadable(format!(
                    "#{} is typed by more than one type object",
                    id.0
                )));
            }
        };
        let mut value = None;
        let mut user_defined = None;
        if let Some(type_id) = type_object {
            let declared = self.designation(type_id, "PredefinedType").ok().flatten();
            let custom = || match self.designation(type_id, "ElementType") {
                Ok(text) => text,
                Err(()) => self.designation(type_id, "ProcessType").ok().flatten(),
            };
            let (designation, custom_used) = match declared.as_deref() {
                Some("USERDEFINED") => (custom(), true),
                None => {
                    let text = custom();
                    let used = text.as_deref().is_some_and(|text| !text.is_empty());
                    (text, used)
                }
                Some(_) => (declared.clone(), false),
            };
            if declared.as_deref() == Some("USERDEFINED") || custom_used {
                user_defined = Some(true);
            }
            if let Some(designation) = designation.filter(|d| !d.is_empty() && d != "NOTDEFINED") {
                value = Some(designation);
                user_defined.get_or_insert(false);
            }
        }
        if value.is_none() {
            let declared = own("PredefinedType");
            value = match declared.as_deref() {
                Some("USERDEFINED") | None => own("ObjectType"),
                Some(_) => declared.clone(),
            };
            if user_defined.is_none() {
                user_defined = Some(match declared.as_deref() {
                    Some("USERDEFINED") => true,
                    None => own("ObjectType").is_some_and(|text| !text.is_empty()),
                    Some(_) => false,
                });
            }
        }
        Ok(ResolvedPredefinedType {
            value,
            user_defined: user_defined.unwrap_or(false),
            evidence: Evidence::exact(
                self.snapshots[0].source().clone(),
                self.locator(format_args!("predefined-type:#{}", id.0)),
            ),
        })
    }

    fn attribute(
        &self,
        object: &ObjectId,
        name: &str,
    ) -> Result<ResolvedAttribute, AttributeError> {
        let id = object
            .local_id
            .strip_prefix('#')
            .and_then(|digits| digits.parse::<u64>().ok())
            .map(EntityId)
            .ok_or_else(|| AttributeError::UnknownObject(object.clone()))?;
        let entity = self
            .model
            .get(id)
            .ok_or_else(|| AttributeError::UnknownObject(object.clone()))?;
        let class = entity.type_name.as_ref();
        let definitions = self.release.schema.attributes(class);
        let Some(slot) = definitions
            .iter()
            .position(|attribute| attribute.name == name)
        else {
            return Err(AttributeError::UnknownAttribute {
                class: class.to_owned(),
                attribute: name.to_owned(),
            });
        };
        let raw = entity.attribute(slot).ok_or_else(|| {
            AttributeError::Unreadable(format!("#{} has no slot for `{name}`", id.0))
        })?;
        let definition = definitions[slot];
        let value = if definition.aggregate {
            match raw {
                Value::Null => AttributeValue::Unset,
                Value::List(items) if items.is_empty() => AttributeValue::Unset,
                Value::List(_) => AttributeValue::Structured,
                Value::Derived => {
                    return Err(AttributeError::Unsupported(
                        "a derived attribute has no stated value".into(),
                    ));
                }
                _ => {
                    return Err(AttributeError::Unreadable(format!(
                        "#{} states a non-aggregate for aggregate `{name}`",
                        id.0
                    )));
                }
            }
        } else {
            self.scalar(raw, &definition.type_name)?
        };
        Ok(ResolvedAttribute {
            value,
            evidence: Evidence::exact(
                self.snapshots[0].source().clone(),
                self.locator(format_args!("attribute:#{}.{name}", id.0)),
            ),
        })
    }
}

fn unit(declared: &str) -> AttributeError {
    AttributeError::Unsupported(format!("{declared} is a measure; its unit is not read"))
}

/// Type objects of every occurrence named by an `IfcRelDefinesByType`.
fn type_index(release: Release, model: &Model) -> BTreeMap<EntityId, Vec<EntityId>> {
    let schema = release.schema;
    let names = schema.attribute_names("IfcRelDefinesByType");
    let slot = |wanted: &str| names.iter().position(|name| *name == wanted);
    let (Some(objects), Some(relating)) = (slot("RelatedObjects"), slot("RelatingType")) else {
        return BTreeMap::new();
    };
    let mut index: BTreeMap<EntityId, Vec<EntityId>> = BTreeMap::new();
    for (_, relationship) in model.iter() {
        if !relationship
            .type_name
            .eq_ignore_ascii_case("IFCRELDEFINESBYTYPE")
        {
            continue;
        }
        let Some(Value::Ref(type_id)) = relationship.attribute(relating) else {
            continue;
        };
        if let Some(Value::List(related)) = relationship.attribute(objects) {
            for item in related {
                if let Value::Ref(occurrence) = item {
                    let types = index.entry(*occurrence).or_default();
                    if !types.contains(type_id) {
                        types.push(*type_id);
                    }
                }
            }
        }
    }
    index
}
