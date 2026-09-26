//! Assigned materials, through `ifc-material`.
//!
//! The upstream crate resolves the one `IfcRelAssociatesMaterial` that
//! applies to an object, directly or through its type, and reads the
//! material resource entities. It reads IFC4 slots only, so an IFC2X3 model
//! is refused rather than read with the wrong table.
//!
//! A layer or profile set usage stands for its set. The names reported are
//! those IDS matches against: the composition's own name, each part's name
//! and category, and each part's material's name and category.

use std::collections::BTreeSet;
use std::sync::Arc;

use axioval_engine::{MaterialError, MaterialService, ResolvedMaterial, SourceSnapshot};
use axioval_ir::{Evidence, ObjectId};
use ifc_material::{
    MaterialDefinition, MaterialUsageDefinition, MaterialView, ResolvedMaterialSelect,
};
use ifc_model::{EntityId, Model};

use crate::release::Release;

pub(crate) struct IfcMaterialService {
    release: Release,
    model: Arc<Model>,
    snapshots: Arc<[SourceSnapshot]>,
}

impl IfcMaterialService {
    pub(crate) fn new(
        release: Release,
        model: Arc<Model>,
        snapshots: Arc<[SourceSnapshot]>,
    ) -> Self {
        Self {
            release,
            model,
            snapshots,
        }
    }
}

type Names = BTreeSet<String>;

fn add(names: &mut Names, name: Option<&str>) {
    if let Some(name) = name.filter(|name| !name.is_empty()) {
        names.insert(name.to_owned());
    }
}

fn unreadable(error: impl std::fmt::Display) -> MaterialError {
    MaterialError::Unreadable(error.to_string())
}

/// The names of a material, a set of parts, or one part.
fn collect(view: MaterialView<'_>, id: EntityId, names: &mut Names) -> Result<(), MaterialError> {
    select_names(
        view,
        view.resolve_material_select(id).map_err(unreadable)?,
        names,
    )
}

fn select_names(
    view: MaterialView<'_>,
    select: ResolvedMaterialSelect<'_>,
    names: &mut Names,
) -> Result<(), MaterialError> {
    match select {
        ResolvedMaterialSelect::Definition(definition) => definition_names(view, definition, names),
        ResolvedMaterialSelect::List(list) => {
            for material in list.material_ids().map_err(unreadable)? {
                collect(view, material, names)?;
            }
            Ok(())
        }
        ResolvedMaterialSelect::Usage(usage) => {
            let set = match usage {
                MaterialUsageDefinition::LayerSet(usage) => usage.layer_set_id(),
                MaterialUsageDefinition::ProfileSet(usage) => usage.profile_set_id(),
                MaterialUsageDefinition::ProfileSetTapering(usage) => usage.profile_set_id(),
            }
            .map_err(unreadable)?;
            collect(view, set, names)
        }
    }
}

fn definition_names(
    view: MaterialView<'_>,
    definition: MaterialDefinition<'_>,
    names: &mut Names,
) -> Result<(), MaterialError> {
    // A part names itself and its material.
    let part = |names: &mut Names,
                name: Option<&str>,
                category: Option<&str>,
                material: Option<EntityId>|
     -> Result<(), MaterialError> {
        add(names, name);
        add(names, category);
        material.map_or(Ok(()), |material| collect(view, material, names))
    };
    match definition {
        MaterialDefinition::Material(material) => {
            add(names, Some(material.name().map_err(unreadable)?));
            add(names, material.category().map_err(unreadable)?);
            Ok(())
        }
        MaterialDefinition::Layer(layer) => part(
            names,
            layer.name().map_err(unreadable)?,
            layer.category().map_err(unreadable)?,
            layer.material_id().map_err(unreadable)?,
        ),
        MaterialDefinition::LayerWithOffsets(layer) => part(
            names,
            layer.name().map_err(unreadable)?,
            layer.category().map_err(unreadable)?,
            layer.material_id().map_err(unreadable)?,
        ),
        MaterialDefinition::Profile(profile) => part(
            names,
            profile.name().map_err(unreadable)?,
            profile.category().map_err(unreadable)?,
            profile.material_id().map_err(unreadable)?,
        ),
        MaterialDefinition::ProfileWithOffsets(profile) => part(
            names,
            profile.name().map_err(unreadable)?,
            profile.category().map_err(unreadable)?,
            profile.material_id().map_err(unreadable)?,
        ),
        MaterialDefinition::Constituent(constituent) => part(
            names,
            constituent.name().map_err(unreadable)?,
            constituent.category().map_err(unreadable)?,
            Some(constituent.material_id().map_err(unreadable)?),
        ),
        MaterialDefinition::LayerSet(set) => {
            add(names, set.name().map_err(unreadable)?);
            for layer in set.layer_ids().map_err(unreadable)? {
                collect(view, layer, names)?;
            }
            Ok(())
        }
        MaterialDefinition::ProfileSet(set) => {
            add(names, set.name().map_err(unreadable)?);
            for profile in set.profile_ids().map_err(unreadable)? {
                collect(view, profile, names)?;
            }
            Ok(())
        }
        MaterialDefinition::ConstituentSet(set) => {
            add(names, set.name().map_err(unreadable)?);
            for constituent in set
                .constituent_ids()
                .map_err(unreadable)?
                .unwrap_or_default()
            {
                collect(view, constituent, names)?;
            }
            Ok(())
        }
    }
}

impl MaterialService for IfcMaterialService {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn material(&self, object: &ObjectId) -> Result<Option<ResolvedMaterial>, MaterialError> {
        if self.release.label != "IFC4" {
            return Err(MaterialError::Unsupported(format!(
                "materials are read for IFC4 only; this model is {}",
                self.release.label
            )));
        }
        let id = object
            .local_id
            .strip_prefix('#')
            .and_then(|digits| digits.parse::<u64>().ok())
            .map(EntityId)
            .ok_or_else(|| MaterialError::UnknownObject(object.clone()))?;
        let view = MaterialView::new(&self.model);
        let Some(assigned) = view.assigned_material(id).map_err(unreadable)? else {
            return Ok(None);
        };
        let mut names = Names::new();
        select_names(view, assigned.material, &mut names)?;
        Ok(Some(ResolvedMaterial {
            names: names.into_iter().collect(),
            evidence: Evidence::exact(
                self.snapshots[0].source().clone(),
                format!(
                    "ifc:{}:material:#{}:#{}",
                    self.snapshots[0].fingerprint(),
                    id.0,
                    assigned.assignment.id().0
                ),
            ),
        }))
    }
}
