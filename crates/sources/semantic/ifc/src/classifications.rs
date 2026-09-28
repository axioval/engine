//! Classification assignments of one IFC model, read through `ifc-classification`.
//!
//! Every assignment the object carries directly or inherits from its type is
//! resolved to its system and its chain of codes. The release-specific
//! reading (IFC2X3 `ItemReference` versus IFC4 `Identification`, notations,
//! structured edition dates) is `ifc-classification`'s; this module only maps
//! its answers onto the engine's source-neutral contract.
//!
//! A resource object (a material, a person, a document) is no `IfcRoot` and
//! cannot be named by `IfcRelAssociatesClassification`. It is classified
//! through the resource-level relationships instead: an IFC4
//! `IfcExternalReferenceRelationship` whose reference is an
//! `IfcClassificationReference` (`ifc-classification`), and a material's
//! `IfcMaterialClassificationRelationship` (`ifc-material`). Both are read
//! for every resource object, and a malformed one refuses every resource
//! object's answer, never an object's.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use axioval_engine::{
    ClassificationAssignment, ClassificationError, ClassificationService, SourceSnapshot,
};
use axioval_ir::ObjectId;
use ifc_classification::{ClassificationView, classification_schema};
use ifc_material::MaterialView;
use ifc_model::{Budget, EntityId, Model};

use crate::release::Release;
use crate::resources::is_object;

type Answer = Result<Vec<ClassificationAssignment>, ClassificationError>;

pub(crate) struct IfcClassificationService {
    release: Release,
    model: Arc<Model>,
    snapshots: Arc<[SourceSnapshot]>,
    /// Resolved assignments per classification item: many objects share one
    /// reference, and its hierarchy walk is the same for all of them.
    items: Mutex<BTreeMap<EntityId, Result<ClassificationAssignment, String>>>,
    /// The classification items each resource object is related to.
    resources: OnceLock<Result<BTreeMap<EntityId, Vec<EntityId>>, String>>,
}

impl IfcClassificationService {
    pub(crate) fn new(
        release: Release,
        model: Arc<Model>,
        snapshots: Arc<[SourceSnapshot]>,
    ) -> Self {
        Self {
            release,
            model,
            snapshots,
            items: Mutex::new(BTreeMap::new()),
            resources: OnceLock::new(),
        }
    }

    /// The classification items the resource-level relationships relate
    /// `resource` to, in model order.
    fn resource_items(&self, resource: EntityId) -> Result<Vec<EntityId>, ClassificationError> {
        let index = self
            .resources
            .get_or_init(|| resource_index(self.release, &self.model))
            .as_ref()
            .map_err(|message| ClassificationError::Unreadable(message.clone()))?;
        Ok(index.get(&resource).cloned().unwrap_or_default())
    }

    fn entity(&self, object: &ObjectId) -> Result<EntityId, ClassificationError> {
        let id = object
            .local_id
            .strip_prefix('#')
            .and_then(|digits| digits.parse::<u64>().ok())
            .map(EntityId)
            .ok_or_else(|| ClassificationError::UnknownObject(object.clone()))?;
        if self.model.get(id).is_none() {
            return Err(ClassificationError::UnknownObject(object.clone()));
        }
        Ok(id)
    }

    fn item(&self, item: EntityId) -> Result<ClassificationAssignment, ClassificationError> {
        let mut cache = self.items.lock().map_err(|_| {
            ClassificationError::Unreadable("classification cache lock is poisoned".into())
        })?;
        cache
            .entry(item)
            .or_insert_with(|| resolve_item(&self.model, item))
            .clone()
            .map_err(ClassificationError::Unreadable)
    }
}

impl ClassificationService for IfcClassificationService {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn classifications(&self, object: &ObjectId) -> Answer {
        let id = self.entity(object)?;
        // `ifc-classification` (≥ 0.2.2) binds the release the header
        // declares, IFC4X3 included. Should it ever bind another release
        // than the session's, it would read with that release's table:
        // refused, never answered from another release's schema.
        let bound = classification_schema(&self.model)
            .map_err(|error| ClassificationError::Unreadable(error.to_string()))?;
        if bound != self.release.version {
            return Err(ClassificationError::Unreadable(format!(
                "the IFC classification library reads this {} model as {bound:?}, \
                 so its classifications are not read exactly",
                self.release.label
            )));
        }
        let effective = ClassificationView::new(&self.model)
            .effective_classifications(id)
            .map_err(|error| ClassificationError::Unreadable(error.to_string()))?;
        let mut items = Vec::new();
        for assignment in effective.occurrence.iter().chain(&effective.inherited) {
            items.push(
                assignment
                    .relating_classification_id()
                    .map_err(|error| ClassificationError::Unreadable(error.to_string()))?,
            );
        }
        let entity = self
            .model
            .get(id)
            .ok_or_else(|| ClassificationError::UnknownObject(object.clone()))?;
        if !is_object(self.release, &entity.type_name) {
            items.extend(self.resource_items(id)?);
        }
        let mut out = Vec::new();
        for item in items {
            let resolved = self.item(item)?;
            if !out.contains(&resolved) {
                out.push(resolved);
            }
        }
        Ok(out)
    }
}

/// Every resource object's classification items: the `IfcClassificationReference`
/// of each `IfcExternalReferenceRelationship` naming it (IFC4 onwards; other
/// external references classify nothing), and the classifications of each
/// `IfcMaterialClassificationRelationship` classifying it.
fn resource_index(
    release: Release,
    model: &Model,
) -> Result<BTreeMap<EntityId, Vec<EntityId>>, String> {
    let mut index: BTreeMap<EntityId, Vec<EntityId>> = BTreeMap::new();
    let schema = release.schema;
    if schema.entity("IfcExternalReferenceRelationship").is_some() {
        let view = ClassificationView::new(model);
        for relationship in view.external_reference_relationships() {
            let relationship = view
                .external_reference_relationship(relationship.id())
                .map_err(|error| error.to_string())?;
            let reference = relationship
                .relating_reference()
                .map_err(|error| error.to_string())?;
            let classifies = model
                .get(reference)
                .is_some_and(|entity| schema.is_a(&entity.type_name, "IFCCLASSIFICATIONREFERENCE"));
            if !classifies {
                continue;
            }
            for resource in relationship
                .related_resources()
                .map_err(|error| error.to_string())?
            {
                index.entry(resource).or_default().push(reference);
            }
        }
    }
    let materials = MaterialView::new(model);
    for relationship in materials.classification_relationships() {
        let material = relationship
            .material_id()
            .map_err(|error| error.to_string())?;
        let items = relationship
            .classification_ids()
            .map_err(|error| error.to_string())?;
        index.entry(material).or_default().extend(items);
    }
    Ok(index)
}

/// System and code chain of one `RelatingClassification` target.
fn resolve_item(model: &Model, item: EntityId) -> Result<ClassificationAssignment, String> {
    let view = ClassificationView::new(model);
    let entity = model
        .get(item)
        .ok_or_else(|| format!("classification item {item} is not in the model"))?;
    if entity.is_type("IFCCLASSIFICATIONREFERENCE") {
        let hierarchy = view
            .hierarchy_from(item, Budget::DEFAULT)
            .map_err(|error| error.to_string())?;
        let codes = hierarchy
            .references
            .iter()
            .map(|reference| {
                reference
                    .identification()
                    .map(|code| code.map(str::to_owned))
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let system = hierarchy
            .system
            .map(|system| system.name().map(str::to_owned))
            .transpose()
            .map_err(|error| error.to_string())?;
        return Ok(ClassificationAssignment { system, codes });
    }
    if entity.is_type("IFCCLASSIFICATION") {
        // Assigned to a whole system: it names no code.
        let system = view
            .systems()
            .find(|system| system.id() == item)
            .ok_or_else(|| format!("classification {item} cannot be read"))?
            .name()
            .map_err(|error| error.to_string())?
            .to_owned();
        return Ok(ClassificationAssignment {
            system: Some(system),
            codes: Vec::new(),
        });
    }
    if entity.is_type("IFCCLASSIFICATIONNOTATION") {
        // IFC2X3 notations carry facet values but no link to their system,
        // so a system-qualified selector cannot decide them.
        let values = view
            .notation_values(item)
            .map_err(|error| error.to_string())?;
        return Ok(ClassificationAssignment {
            system: None,
            codes: vec![Some(values.concat())],
        });
    }
    Err(format!(
        "classification item {item} is a {}, not a classification",
        entity.type_name
    ))
}
