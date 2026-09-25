//! Classification assignments of one IFC model, read through `ifc-classification`.
//!
//! Every assignment the object carries directly or inherits from its type is
//! resolved to its system and its chain of codes. The release-specific
//! reading (IFC2X3 `ItemReference` versus IFC4 `Identification`, notations,
//! structured edition dates) is `ifc-classification`'s; this module only maps
//! its answers onto the engine's source-neutral contract.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axioval_engine::{
    ClassificationAssignment, ClassificationError, ClassificationService, SourceSnapshot,
};
use axioval_ir::ObjectId;
use ifc_classification::ClassificationView;
use ifc_model::{Budget, EntityId, Model};

type Answer = Result<Vec<ClassificationAssignment>, ClassificationError>;

pub(crate) struct IfcClassificationService {
    model: Arc<Model>,
    snapshots: Arc<[SourceSnapshot]>,
    /// Resolved assignments per classification item: many objects share one
    /// reference, and its hierarchy walk is the same for all of them.
    items: Mutex<BTreeMap<EntityId, Result<ClassificationAssignment, String>>>,
}

impl IfcClassificationService {
    pub(crate) fn new(model: Arc<Model>, snapshots: Arc<[SourceSnapshot]>) -> Self {
        Self {
            model,
            snapshots,
            items: Mutex::new(BTreeMap::new()),
        }
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
        let effective = ClassificationView::new(&self.model)
            .effective_classifications(id)
            .map_err(|error| ClassificationError::Unreadable(error.to_string()))?;
        let mut out = Vec::new();
        for assignment in effective.occurrence.iter().chain(&effective.inherited) {
            let item = assignment
                .relating_classification_id()
                .map_err(|error| ClassificationError::Unreadable(error.to_string()))?;
            let resolved = self.item(item)?;
            if !out.contains(&resolved) {
                out.push(resolved);
            }
        }
        Ok(out)
    }
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
