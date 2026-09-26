//! Entity attributes read as properties in the reserved attribute sets.
//!
//! Much of what a checker asks about an IFC object is not in a property set
//! but in the entity itself: a space's number is `IfcSpace.Name`, its name is
//! `LongName`, a door's construction type is the `Name` of its
//! `IfcDoorType`. A request in [`ATTRIBUTE_SET`] reads the object's own
//! attribute by its schema name; one in [`TYPE_ATTRIBUTE_SET`] reads the
//! attribute of the type object assigned through `IfcRelDefinesByType`.
//!
//! Only scalar values that mean the same in every file are answered: text,
//! enumerations, booleans, integers and unit-free reals. A measure
//! (`IfcLengthMeasure`, ...) is in the project's units and is refused until
//! this adapter converts units, as is a reference or an aggregate. An unset
//! attribute (`$`), an attribute the entity does not declare, and an object
//! with no type are exact absences; an object typed twice is a conflict.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use axioval_engine::PropertyResolutionError;
use axioval_ir::{ATTRIBUTE_SET, PropertyValue, TYPE_ATTRIBUTE_SET};
use ifc_model::{EntityId, Model, Value};
use ifc_schema::{Schema, TypeKind};

use crate::release::Release;

/// A present attribute value and the locator detail that proves it.
pub(crate) struct AttributeValue {
    pub(crate) value: PropertyValue,
    pub(crate) detail: String,
}

/// Type objects assigned to each object, with the assigning relationship.
type TypeIndex = BTreeMap<EntityId, Vec<(EntityId, EntityId)>>;

/// Answers attribute requests for one model.
pub(crate) struct Attributes {
    release: Release,
    types: OnceLock<Result<TypeIndex, String>>,
}

impl Attributes {
    pub(crate) fn new(release: Release) -> Self {
        Self {
            release,
            types: OnceLock::new(),
        }
    }

    /// Reads `name` in the reserved `set` for `object`; `Ok(None)` is exact absence.
    pub(crate) fn resolve(
        &self,
        model: &Model,
        object: EntityId,
        set: &str,
        name: &str,
    ) -> Result<Option<AttributeValue>, PropertyResolutionError> {
        let schema = self.release.schema;
        if set == ATTRIBUTE_SET {
            return read(schema, model, object, name).map(|value| {
                value.map(|value| AttributeValue {
                    value,
                    detail: format!("attribute:{object}:{name}"),
                })
            });
        }
        debug_assert_eq!(set, TYPE_ATTRIBUTE_SET);
        let types = self
            .types
            .get_or_init(|| index_types(schema, model))
            .as_ref()
            .map_err(|message| PropertyResolutionError::Incomplete(message.clone()))?;
        match types.get(&object).map(Vec::as_slice) {
            None | Some([]) => Ok(None),
            Some([(type_object, relationship)]) => {
                read(schema, model, *type_object, name).map(|value| {
                    value.map(|value| AttributeValue {
                        value,
                        detail: format!("type-attribute:{relationship}:{type_object}:{name}"),
                    })
                })
            }
            Some(several) => Err(PropertyResolutionError::Conflicting(format!(
                "{object} is typed by {} type objects ({})",
                several.len(),
                several
                    .iter()
                    .map(|(type_object, _)| type_object.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    }
}

/// One attribute of one entity, by schema name (ASCII case-insensitive).
fn read(
    schema: &Schema,
    model: &Model,
    id: EntityId,
    name: &str,
) -> Result<Option<PropertyValue>, PropertyResolutionError> {
    let entity = model
        .get(id)
        .ok_or(PropertyResolutionError::InvalidRequest)?;
    let attributes = schema.attributes(&entity.type_name);
    let Some((slot, attribute)) = attributes
        .iter()
        .enumerate()
        .find(|(_, attribute)| attribute.name.eq_ignore_ascii_case(name))
    else {
        // The entity type declares no such attribute: it cannot have it.
        return Ok(None);
    };
    let unsupported = |why: &str| {
        Err(PropertyResolutionError::Unavailable(format!(
            "{id}.{} {why}",
            attribute.name
        )))
    };
    if attribute.aggregate {
        return unsupported("is an aggregate, not a scalar value");
    }
    match entity.attribute(slot) {
        None => Err(PropertyResolutionError::Incomplete(format!(
            "{id} has no slot for {}",
            attribute.name
        ))),
        Some(Value::Null) => Ok(None),
        Some(Value::Derived) => unsupported("is derived and holds no stated value"),
        Some(Value::Typed { type_name, value }) => scalar(schema, type_name, value).map_or_else(
            || unsupported("holds a value this adapter cannot read exactly"),
            |value| Ok(Some(value)),
        ),
        Some(value) => scalar(schema, &attribute.type_name, value).map_or_else(
            || unsupported("holds a value this adapter cannot read exactly"),
            |value| Ok(Some(value)),
        ),
    }
}

/// A scalar value of declared type `type_name`, when it reads the same in every file.
fn scalar(schema: &Schema, type_name: &str, value: &Value) -> Option<PropertyValue> {
    // Measures are stated in the project's units; reading them as bare
    // numbers would compare millimetres with metres.
    if type_name.to_ascii_uppercase().ends_with("MEASURE") {
        return None;
    }
    if schema.entity(type_name).is_some() {
        return None;
    }
    let kind = schema
        .type_def(type_name)
        .map(|definition| &definition.kind);
    if let Some(TypeKind::Enumeration(_)) = kind {
        return match value {
            Value::Enum(item) => Some(PropertyValue::String(item.to_string())),
            _ => None,
        };
    }
    if let Some(TypeKind::Select(_)) = kind {
        // A select value is written typed; an untyped one cannot be placed.
        return None;
    }
    let base = schema.resolve_defined(type_name).to_ascii_uppercase();
    match value {
        Value::Text(text) if base.starts_with("STRING") => {
            Some(PropertyValue::String(text.to_string()))
        }
        Value::Bool(flag) if base == "BOOLEAN" || base == "LOGICAL" => {
            Some(PropertyValue::Boolean(*flag))
        }
        Value::Integer(number) if base == "INTEGER" => Some(PropertyValue::Integer(*number)),
        Value::Real(number) if base == "REAL" && number.is_finite() => {
            Some(PropertyValue::Decimal(*number))
        }
        #[allow(clippy::cast_precision_loss)]
        Value::Integer(number) if base == "REAL" && number.unsigned_abs() <= 1 << 53 => {
            Some(PropertyValue::Decimal(*number as f64))
        }
        _ => None,
    }
}

/// Every `IfcRelDefinesByType` edge, object to (type object, relationship).
fn index_types(schema: &Schema, model: &Model) -> Result<TypeIndex, String> {
    let names = schema.attribute_names("IfcRelDefinesByType");
    let slot = |name: &str| {
        names
            .iter()
            .position(|candidate| *candidate == name)
            .ok_or_else(|| format!("IfcRelDefinesByType declares no {name}"))
    };
    let (related, relating) = (slot("RelatedObjects")?, slot("RelatingType")?);
    let mut index = TypeIndex::new();
    for relationship in model.ids_of_type("IfcRelDefinesByType") {
        let malformed = |what: &str| format!("{relationship} (IfcRelDefinesByType) {what}");
        let entity = model
            .get(*relationship)
            .ok_or_else(|| malformed("is indexed but absent"))?;
        let Some(Value::Ref(type_object)) = entity.attribute(relating) else {
            return Err(malformed("has no RelatingType reference"));
        };
        if model.get(*type_object).is_none() {
            return Err(malformed("references a missing type object"));
        }
        let Some(Value::List(objects)) = entity.attribute(related) else {
            return Err(malformed("has no RelatedObjects list"));
        };
        for object in objects {
            let Value::Ref(object) = object else {
                return Err(malformed("lists a related object that is not a reference"));
            };
            let entry = index.entry(*object).or_default();
            if !entry.iter().any(|(held, _)| held == type_object) {
                entry.push((*type_object, *relationship));
            }
        }
    }
    Ok(index)
}
