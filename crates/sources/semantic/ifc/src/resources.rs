//! Resource objects: the instances a session makes no object of.
//!
//! A session's objects are its occurrences, contexts and type objects
//! (`is_object`). Every other instance (a material, a classification, a
//! relationship, a task time, a surface style) is a resource object, listed
//! by class only when a rule names that class. Its identity is its STEP
//! instance number, source-qualified like an object's, and its GlobalId
//! alias where it is an `IfcRoot` with a valid and unique one (a
//! relationship).

use std::sync::Arc;

use axioval_engine::{ResourceError, ResourceRequest, ResourceService, SourceSnapshot};
use axioval_ir::{ExternalId, Object, ObjectId};
use ifc_model::Model;

use crate::identity::{GlobalIds, IFC_GLOBAL_ID};
use crate::release::Release;

/// The classes whose instances, and whose subclasses' instances, are a
/// session's objects.
pub(crate) const OBJECT_ROOTS: [&str; 3] = ["IFCOBJECT", "IFCCONTEXT", "IFCTYPEOBJECT"];

/// Whether a session makes an object of an instance of `type_name`: an
/// occurrence, a context or a type object of the release.
pub(crate) fn is_object(release: Release, type_name: &str) -> bool {
    OBJECT_ROOTS
        .iter()
        .any(|root| release.schema.is_a(type_name, root))
}

/// Lists one model's resource objects by class.
pub(crate) struct IfcResources {
    release: Release,
    model: Arc<Model>,
    global_ids: Arc<GlobalIds>,
    snapshots: Arc<[SourceSnapshot]>,
}

impl IfcResources {
    pub(crate) fn new(
        release: Release,
        model: Arc<Model>,
        global_ids: Arc<GlobalIds>,
        snapshots: Arc<[SourceSnapshot]>,
    ) -> Self {
        Self {
            release,
            model,
            global_ids,
            snapshots,
        }
    }

    /// Whether `class` names resource objects: the release declares it, it
    /// is no object class, and with subtypes none of its subclasses is one.
    /// IFC inherits singly, so a class has an object subclass exactly when
    /// it is an ancestor of an object root (`IfcRoot`, `IfcObjectDefinition`).
    fn names_resources(&self, class: &str, include_subtypes: bool) -> bool {
        let schema = self.release.schema;
        schema.entity(class).is_some()
            && !is_object(self.release, class)
            && !(include_subtypes && OBJECT_ROOTS.iter().any(|root| schema.is_a(root, class)))
    }
}

impl ResourceService for IfcResources {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn resources(&self, request: &ResourceRequest) -> Result<Vec<Object>, ResourceError> {
        let class = request.class();
        let subtypes = request.include_subtypes();
        if !self.names_resources(class, subtypes) {
            return Ok(Vec::new());
        }
        let schema = self.release.schema;
        let mut listed = Vec::new();
        for (id, entity) in self.model.iter() {
            let of_class = if subtypes {
                schema.is_a(&entity.type_name, class)
            } else {
                entity.type_name.eq_ignore_ascii_case(class)
            };
            if !of_class {
                continue;
            }
            let identity = ObjectId::new(request.source().clone(), id.to_string())
                .map_err(|error| ResourceError::Unreadable(error.to_string()))?;
            let mut object = Object::new(identity, entity.type_name.to_string());
            if let Some(global_id) = self.global_ids.of(id) {
                let alias = ExternalId::new(IFC_GLOBAL_ID, global_id)
                    .map_err(|error| ResourceError::Unreadable(error.to_string()))?;
                object = object.with_external_id(alias);
            }
            listed.push(object);
        }
        listed.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(listed)
    }
}
