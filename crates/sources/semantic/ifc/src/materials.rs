//! Materials read as properties in the reserved material set.
//!
//! `ifc-material` resolves which `IfcRelAssociatesMaterial` applies to an
//! object: the object's own, else its `IfcRelDefinesByType` type object's.
//! This module only projects the resolved material onto the vocabulary of
//! [`MATERIAL_SET`]: a single material, a layer set (directly or through an
//! `IfcMaterialLayerSetUsage`), a constituent set, a profile set (directly or
//! through an `IfcMaterialProfileSetUsage`), or a material list.
//!
//! An object without material is an exact absence of every property. Two
//! assignments at one level, or two type objects, are a conflict. A malformed
//! material, a material reached through a tapering profile usage (two profile
//! sets), and a lone layer, constituent or profile associated directly are
//! refused, never answered in part. Layer thicknesses and constituent
//! fractions go through `measure.rs`, so they are SI values or refused; a
//! unit the file does not resolve refuses that measure alone, since the
//! material's names and members are read exactly without it.
//!
//! `Names` lists every name and category the material goes by, as IDS
//! matches a material value: the material's or set's own, each member's, and
//! each member's material's.
//!
//! IFC2X3 is refused: `ifc-material` 0.2 reads IFC4 slot positions whatever
//! release the file declares (openbimrs/ifc#77).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use axioval_engine::PropertyResolutionError;
use axioval_ir::{
    MATERIAL_CATEGORY, MATERIAL_COUNT, MATERIAL_KIND, MATERIAL_KIND_CONSTITUENT_SET,
    MATERIAL_KIND_LAYER_SET, MATERIAL_KIND_LIST, MATERIAL_KIND_PROFILE_SET, MATERIAL_KIND_SINGLE,
    MATERIAL_NAME, MATERIAL_NAMES, MATERIAL_TOTAL_THICKNESS, PropertyValue,
};
use ifc_material::{
    AssignmentSource, Material, MaterialConstituentSet, MaterialDefinition, MaterialError,
    MaterialLayerSet, MaterialList, MaterialProfileSet, MaterialUsageDefinition, MaterialView,
    ResolvedMaterialSelect,
};
use ifc_model::{EntityId, Model};
use ifc_schema::Schema;

use crate::attributes::AttributeValue;
use crate::measure::si_value;
use crate::release::Release;

/// Every material property of one object, keyed by lower-case name. A
/// measure whose unit cannot be resolved is refused on its own.
type Composition = BTreeMap<String, Result<(PropertyValue, String), PropertyResolutionError>>;

/// The material that applies to one object, as properties and as the
/// `IfcMaterial` instances it is made of.
struct Resolved {
    properties: Composition,
    /// Every `IfcMaterial` the resolved material names, in the source's
    /// order and without repeats: the single material, or each member's.
    materials: Vec<EntityId>,
}

/// Answers material requests for one model, each object resolved once.
pub(crate) struct Materials {
    release: Release,
    resolved: Mutex<BTreeMap<EntityId, Result<Arc<Resolved>, PropertyResolutionError>>>,
}

impl Materials {
    pub(crate) fn new(release: Release) -> Self {
        Self {
            release,
            resolved: Mutex::new(BTreeMap::new()),
        }
    }

    /// Reads `name` in the material set for `object`; `Ok(None)` is exact absence.
    pub(crate) fn resolve(
        &self,
        model: &Model,
        object: EntityId,
        name: &str,
    ) -> Result<Option<AttributeValue>, PropertyResolutionError> {
        let resolved = self.resolved(model, object)?;
        match resolved.properties.get(&name.to_ascii_lowercase()) {
            None => Ok(None),
            Some(Ok((value, detail))) => Ok(Some(AttributeValue {
                value: value.clone(),
                detail: detail.clone(),
            })),
            Some(Err(error)) => Err(error.clone()),
        }
    }

    /// Every `IfcMaterial` the material of `object` is made of; empty when
    /// the object has no material.
    pub(crate) fn material_ids(
        &self,
        model: &Model,
        object: EntityId,
    ) -> Result<Vec<EntityId>, PropertyResolutionError> {
        Ok(self.resolved(model, object)?.materials.clone())
    }

    fn resolved(
        &self,
        model: &Model,
        object: EntityId,
    ) -> Result<Arc<Resolved>, PropertyResolutionError> {
        if self.release.label != "IFC4" {
            return Err(PropertyResolutionError::Unavailable(format!(
                "materials are not read from {} files until ifc-material binds to the \
                 file's release (openbimrs/ifc#77)",
                self.release.label
            )));
        }
        let mut resolved = self.resolved.lock().unwrap_or_else(PoisonError::into_inner);
        resolved
            .entry(object)
            .or_insert_with(|| compose(self.release.schema, model, object).map(Arc::new))
            .clone()
    }
}

/// Collects the properties of the material that applies to `object`.
struct Writer<'s> {
    schema: &'s Schema,
    prefix: String,
    out: Composition,
    materials: Vec<EntityId>,
    /// Every name and category the material goes by, for [`MATERIAL_NAMES`].
    names: BTreeSet<String>,
}

impl Writer<'_> {
    fn put(&mut self, name: &str, value: PropertyValue, holder: impl std::fmt::Display) {
        self.out.insert(
            name.to_ascii_lowercase(),
            Ok((value, format!("{}:{holder}", self.prefix))),
        );
    }

    /// A measure, or its refusal for this property alone: a unit the file
    /// does not resolve leaves the material's names and members readable.
    fn put_measure(
        &mut self,
        name: &str,
        value: Result<PropertyValue, PropertyResolutionError>,
        holder: impl std::fmt::Display,
    ) {
        match value {
            Ok(value) => self.put(name, value, holder),
            Err(error) => {
                self.out.insert(name.to_ascii_lowercase(), Err(error));
            }
        }
    }

    fn text(&mut self, name: &str, value: Option<&str>, holder: impl std::fmt::Display) {
        if let Some(value) = value {
            self.named(Some(value));
            self.put(name, PropertyValue::String(value.to_owned()), holder);
        }
    }

    /// Records a name the material goes by; an empty one names nothing.
    fn named(&mut self, name: Option<&str>) {
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            self.names.insert(name.to_owned());
        }
    }

    /// Writes [`MATERIAL_NAMES`] once every name is recorded.
    fn finish_names(&mut self, holder: impl std::fmt::Display) {
        let names = std::mem::take(&mut self.names)
            .into_iter()
            .map(PropertyValue::String)
            .collect();
        self.put(MATERIAL_NAMES, PropertyValue::List(names), holder);
    }

    /// Records that the material is made of `material`.
    fn material(&mut self, material: EntityId) {
        if !self.materials.contains(&material) {
            self.materials.push(material);
        }
    }

    fn count(&mut self, len: usize, holder: EntityId) -> Result<(), PropertyResolutionError> {
        let len = i64::try_from(len).map_err(|_| PropertyResolutionError::InvalidValue)?;
        self.put(MATERIAL_COUNT, PropertyValue::Integer(len), holder);
        Ok(())
    }

    /// A number of the type `entity.attribute` declares, converted to SI.
    fn measure(
        &self,
        model: &Model,
        entity: &str,
        attribute: &str,
        value: f64,
    ) -> Result<PropertyValue, PropertyResolutionError> {
        let declared = self
            .schema
            .attributes(entity)
            .iter()
            .find(|candidate| candidate.name.eq_ignore_ascii_case(attribute))
            .map(|candidate| candidate.type_name.to_string())
            .ok_or_else(|| {
                PropertyResolutionError::Incomplete(format!("{entity} declares no {attribute}"))
            })?;
        si_value(model, &declared, None, value)?.ok_or_else(|| {
            PropertyResolutionError::Incomplete(format!(
                "{entity}.{attribute} is declared {declared}, not a measure"
            ))
        })
    }
}

fn compose(
    schema: &Schema,
    model: &Model,
    object: EntityId,
) -> Result<Resolved, PropertyResolutionError> {
    let view = MaterialView::new(model);
    let Some(resolved) = view.assigned_material(object).map_err(refusal)? else {
        return Ok(Resolved {
            properties: Composition::new(),
            materials: Vec::new(),
        });
    };
    let provenance = match resolved.source {
        AssignmentSource::Occurrence => "occurrence".to_owned(),
        AssignmentSource::Type(type_object) => format!("type:{type_object}"),
    };
    let mut writer = Writer {
        schema,
        prefix: format!(
            "material:{object}:{provenance}:{}",
            resolved.assignment.id()
        ),
        out: Composition::new(),
        materials: Vec::new(),
        names: BTreeSet::new(),
    };
    match resolved.material {
        ResolvedMaterialSelect::Definition(MaterialDefinition::Material(material)) => {
            writer.material(material.id());
            writer.put(
                MATERIAL_KIND,
                PropertyValue::String(MATERIAL_KIND_SINGLE.into()),
                material.id(),
            );
            writer.text(
                MATERIAL_NAME,
                Some(material.name().map_err(refusal)?),
                material.id(),
            );
            writer.text(
                MATERIAL_CATEGORY,
                material.category().map_err(refusal)?,
                material.id(),
            );
        }
        ResolvedMaterialSelect::Definition(MaterialDefinition::LayerSet(set)) => {
            layer_set(view, &mut writer, set)?;
        }
        ResolvedMaterialSelect::Usage(MaterialUsageDefinition::LayerSet(usage)) => {
            writer.prefix = format!("{}:usage:{}", writer.prefix, usage.id());
            let target = usage.layer_set_id().map_err(refusal)?;
            match view.resolve_material_select(target).map_err(refusal)? {
                ResolvedMaterialSelect::Definition(MaterialDefinition::LayerSet(set)) => {
                    layer_set(view, &mut writer, set)?;
                }
                _ => return Err(wrong(usage.id(), target, "IfcMaterialLayerSet")),
            }
        }
        ResolvedMaterialSelect::Definition(MaterialDefinition::ConstituentSet(set)) => {
            constituent_set(view, &mut writer, set)?;
        }
        ResolvedMaterialSelect::Definition(MaterialDefinition::ProfileSet(set)) => {
            profile_set(view, &mut writer, set)?;
        }
        ResolvedMaterialSelect::Usage(MaterialUsageDefinition::ProfileSet(usage)) => {
            writer.prefix = format!("{}:usage:{}", writer.prefix, usage.id());
            let target = usage.profile_set_id().map_err(refusal)?;
            match view.resolve_material_select(target).map_err(refusal)? {
                ResolvedMaterialSelect::Definition(MaterialDefinition::ProfileSet(set)) => {
                    profile_set(view, &mut writer, set)?;
                }
                _ => return Err(wrong(usage.id(), target, "IfcMaterialProfileSet")),
            }
        }
        ResolvedMaterialSelect::Usage(MaterialUsageDefinition::ProfileSetTapering(usage)) => {
            return Err(PropertyResolutionError::Unavailable(format!(
                "{object} tapers between two profile sets ({}); one material cannot be named",
                usage.id()
            )));
        }
        ResolvedMaterialSelect::List(list) => material_list(view, &mut writer, list)?,
        ResolvedMaterialSelect::Definition(
            MaterialDefinition::Layer(_)
            | MaterialDefinition::LayerWithOffsets(_)
            | MaterialDefinition::Constituent(_)
            | MaterialDefinition::Profile(_)
            | MaterialDefinition::ProfileWithOffsets(_),
        ) => {
            return Err(PropertyResolutionError::Unavailable(format!(
                "{object} is associated with a lone layer, constituent or profile, not a set"
            )));
        }
    }
    writer.finish_names(resolved.assignment.id());
    Ok(Resolved {
        properties: writer.out,
        materials: writer.materials,
    })
}

/// Layers in order, their thicknesses in metres, and the total thickness.
fn layer_set(
    view: MaterialView<'_>,
    writer: &mut Writer<'_>,
    set: MaterialLayerSet<'_>,
) -> Result<(), PropertyResolutionError> {
    let model = view.model();
    writer.put(
        MATERIAL_KIND,
        PropertyValue::String(MATERIAL_KIND_LAYER_SET.into()),
        set.id(),
    );
    writer.text(MATERIAL_NAME, set.name().map_err(refusal)?, set.id());
    let layers = set.layer_ids().map_err(refusal)?;
    writer.count(layers.len(), set.id())?;
    let total = view.total_thickness(set).map_err(refusal)?;
    // The derived total is a sum of layer thicknesses, so it has their type.
    let total = writer.measure(model, "IfcMaterialLayer", "LayerThickness", total);
    writer.put_measure(MATERIAL_TOTAL_THICKNESS, total, set.id());
    for (index, id) in layers.into_iter().enumerate() {
        let member = format!("Layer{}", index + 1);
        macro_rules! read_layer {
            ($layer:expr) => {{
                let layer = $layer;
                let thickness = layer.thickness().map_err(refusal)?;
                let thickness =
                    writer.measure(model, "IfcMaterialLayer", "LayerThickness", thickness);
                writer.put_measure(&format!("{member}.Thickness"), thickness, id);
                writer.text(
                    &format!("{member}.Name"),
                    layer.name().map_err(refusal)?,
                    id,
                );
                writer.text(
                    &format!("{member}.Category"),
                    layer.category().map_err(refusal)?,
                    id,
                );
                layer.material_id().map_err(refusal)?
            }};
        }
        let material = match view.resolve_material_select(id).map_err(refusal)? {
            ResolvedMaterialSelect::Definition(MaterialDefinition::Layer(layer)) => {
                read_layer!(layer)
            }
            ResolvedMaterialSelect::Definition(MaterialDefinition::LayerWithOffsets(layer)) => {
                read_layer!(layer)
            }
            _ => return Err(wrong(set.id(), id, "IfcMaterialLayer")),
        };
        if let Some(material) = material {
            writer.material(material);
            let name = material_name(view, id, material)?;
            writer.named(material_category(view, id, material)?);
            writer.text(
                &format!("{member}.Material"),
                Some(name),
                format_args!("{id}/{material}"),
            );
        }
    }
    Ok(())
}

/// Constituents in order, with their materials and fractions.
fn constituent_set(
    view: MaterialView<'_>,
    writer: &mut Writer<'_>,
    set: MaterialConstituentSet<'_>,
) -> Result<(), PropertyResolutionError> {
    writer.put(
        MATERIAL_KIND,
        PropertyValue::String(MATERIAL_KIND_CONSTITUENT_SET.into()),
        set.id(),
    );
    writer.text(MATERIAL_NAME, set.name().map_err(refusal)?, set.id());
    let constituents = set.constituent_ids().map_err(refusal)?.unwrap_or_default();
    writer.count(constituents.len(), set.id())?;
    for (index, id) in constituents.into_iter().enumerate() {
        let member = format!("Constituent{}", index + 1);
        let ResolvedMaterialSelect::Definition(MaterialDefinition::Constituent(constituent)) =
            view.resolve_material_select(id).map_err(refusal)?
        else {
            return Err(wrong(set.id(), id, "IfcMaterialConstituent"));
        };
        writer.text(
            &format!("{member}.Name"),
            constituent.name().map_err(refusal)?,
            id,
        );
        writer.text(
            &format!("{member}.Category"),
            constituent.category().map_err(refusal)?,
            id,
        );
        if let Some(fraction) = constituent.fraction().map_err(refusal)? {
            let fraction =
                writer.measure(view.model(), "IfcMaterialConstituent", "Fraction", fraction);
            writer.put_measure(&format!("{member}.Fraction"), fraction, id);
        }
        let material = constituent.material_id().map_err(refusal)?;
        writer.material(material);
        let name = material_name(view, id, material)?;
        writer.named(material_category(view, id, material)?);
        writer.text(
            &format!("{member}.Material"),
            Some(name),
            format_args!("{id}/{material}"),
        );
    }
    Ok(())
}

/// Profiles in order, with their materials.
fn profile_set(
    view: MaterialView<'_>,
    writer: &mut Writer<'_>,
    set: MaterialProfileSet<'_>,
) -> Result<(), PropertyResolutionError> {
    writer.put(
        MATERIAL_KIND,
        PropertyValue::String(MATERIAL_KIND_PROFILE_SET.into()),
        set.id(),
    );
    writer.text(MATERIAL_NAME, set.name().map_err(refusal)?, set.id());
    let profiles = set.profile_ids().map_err(refusal)?;
    writer.count(profiles.len(), set.id())?;
    for (index, id) in profiles.into_iter().enumerate() {
        let member = format!("Profile{}", index + 1);
        macro_rules! read_profile {
            ($profile:expr) => {{
                let profile = $profile;
                writer.text(
                    &format!("{member}.Name"),
                    profile.name().map_err(refusal)?,
                    id,
                );
                writer.text(
                    &format!("{member}.Category"),
                    profile.category().map_err(refusal)?,
                    id,
                );
                profile.material_id().map_err(refusal)?
            }};
        }
        let material = match view.resolve_material_select(id).map_err(refusal)? {
            ResolvedMaterialSelect::Definition(MaterialDefinition::Profile(profile)) => {
                read_profile!(profile)
            }
            ResolvedMaterialSelect::Definition(MaterialDefinition::ProfileWithOffsets(profile)) => {
                read_profile!(profile)
            }
            _ => return Err(wrong(set.id(), id, "IfcMaterialProfile")),
        };
        if let Some(material) = material {
            writer.material(material);
            let name = material_name(view, id, material)?;
            writer.named(material_category(view, id, material)?);
            writer.text(
                &format!("{member}.Material"),
                Some(name),
                format_args!("{id}/{material}"),
            );
        }
    }
    Ok(())
}

/// Listed materials in order.
fn material_list(
    view: MaterialView<'_>,
    writer: &mut Writer<'_>,
    list: MaterialList<'_>,
) -> Result<(), PropertyResolutionError> {
    writer.put(
        MATERIAL_KIND,
        PropertyValue::String(MATERIAL_KIND_LIST.into()),
        list.id(),
    );
    let materials = list.material_ids().map_err(refusal)?;
    writer.count(materials.len(), list.id())?;
    for (index, id) in materials.into_iter().enumerate() {
        let member = format!("Material{}", index + 1);
        let material = material(view, list.id(), id)?;
        writer.material(id);
        writer.text(
            &format!("{member}.Name"),
            Some(material.name().map_err(refusal)?),
            id,
        );
        writer.text(
            &format!("{member}.Category"),
            material.category().map_err(refusal)?,
            id,
        );
    }
    Ok(())
}

/// The `IfcMaterial` `holder` references as `id`.
fn material(
    view: MaterialView<'_>,
    holder: EntityId,
    id: EntityId,
) -> Result<Material<'_>, PropertyResolutionError> {
    let entity = view.model().get(id).ok_or_else(|| {
        PropertyResolutionError::Incomplete(format!("{holder} references missing {id}"))
    })?;
    Material::try_new(id, entity).map_err(|_| wrong(holder, id, "IfcMaterial"))
}

fn material_name(
    view: MaterialView<'_>,
    holder: EntityId,
    id: EntityId,
) -> Result<&str, PropertyResolutionError> {
    material(view, holder, id)?.name().map_err(refusal)
}

fn material_category(
    view: MaterialView<'_>,
    holder: EntityId,
    id: EntityId,
) -> Result<Option<&str>, PropertyResolutionError> {
    material(view, holder, id)?.category().map_err(refusal)
}

fn wrong(holder: EntityId, target: EntityId, expected: &str) -> PropertyResolutionError {
    PropertyResolutionError::Incomplete(format!("{holder} references {target}, not an {expected}"))
}

/// Ambiguity is a conflict; anything malformed leaves the answer incomplete.
#[allow(clippy::needless_pass_by_value)] // Passed to `map_err` by name.
fn refusal(error: MaterialError) -> PropertyResolutionError {
    match error {
        MaterialError::AmbiguousAssignment { .. } | MaterialError::AmbiguousType { .. } => {
            PropertyResolutionError::Conflicting(error.to_string())
        }
        _ => PropertyResolutionError::Incomplete(error.to_string()),
    }
}
