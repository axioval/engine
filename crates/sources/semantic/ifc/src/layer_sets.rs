//! Material layers of building elements, read from the
//! `IfcMaterialLayerSet` associated with them (#334).
//!
//! `ifc-material` resolves which `IfcRelAssociatesMaterial` applies to an
//! element, as for `axioval:material` (`materials.rs`): the element's own,
//! else its type object's. A layer set reached through an
//! `IfcMaterialLayerSetUsage` states the direction its layers stack in:
//! `LayerSetDirection` names an axis of the element's `ObjectPlacement`
//! (`AXIS1` to `AXIS3`) and `DirectionSense` whether the layers run along
//! it or against it, the first layer at the start. A layer set associated
//! without a usage states no direction, so none is given.
//!
//! Thicknesses go through `measure.rs`: SI values, or refused for a unit
//! the file does not resolve, that layer's thickness alone. An element
//! without material has no layers; any other material (a single material,
//! a constituent, profile set or list, a tapering profile usage) is
//! refused by name, never read as layers. Two assignments at one level are
//! a conflict.

use std::sync::Arc;

use axioval_engine::{
    LayerAxis, LayerDirection, LayerSense, MaterialLayer, MaterialLayerError, MaterialLayerService,
    MaterialLayers, SourceSnapshot,
};
use axioval_ir::{Evidence, ObjectId, PropertyValue};
use ifc_material::{
    AssignmentSource, DirectionSense, LayerSetDirection, LogicalValue, Material,
    MaterialDefinition, MaterialError, MaterialLayerSet, MaterialUsageDefinition, MaterialView,
    ResolvedMaterialSelect,
};
use ifc_model::{EntityId, Model};

use crate::measure::si_value;
use crate::release::Release;

pub(crate) struct IfcMaterialLayers {
    release: Release,
    model: Arc<Model>,
    snapshots: Arc<[SourceSnapshot]>,
}

impl IfcMaterialLayers {
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

    fn entity(&self, object: &ObjectId) -> Result<EntityId, MaterialLayerError> {
        let id = object
            .local_id
            .strip_prefix('#')
            .and_then(|digits| digits.parse::<u64>().ok())
            .map(EntityId)
            .ok_or_else(|| MaterialLayerError::UnknownObject(object.clone()))?;
        if self.model.get(id).is_none() {
            return Err(MaterialLayerError::UnknownObject(object.clone()));
        }
        Ok(id)
    }

    /// The thickness of a layer in metres, or why its unit cannot be read.
    fn thickness(&self, value: f64) -> Result<f64, String> {
        let declared = self
            .release
            .schema
            .attributes("IfcMaterialLayer")
            .iter()
            .find(|attribute| attribute.name.eq_ignore_ascii_case("LayerThickness"))
            .map(|attribute| attribute.type_name.clone())
            .ok_or("IfcMaterialLayer declares no LayerThickness")?;
        match si_value(&self.model, &declared, None, value) {
            Ok(Some(PropertyValue::Quantity { value, .. })) => Ok(value),
            Ok(_) => Err(format!(
                "LayerThickness is declared {declared}, not a length"
            )),
            Err(error) => Err(error.to_string()),
        }
    }

    fn layers(
        &self,
        object: &ObjectId,
        id: EntityId,
    ) -> Result<(Vec<MaterialLayer>, Option<LayerDirection>, String), MaterialLayerError> {
        let view = MaterialView::new(&self.model);
        let Some(resolved) = view.assigned_material(id).map_err(refusal)? else {
            return Ok((Vec::new(), None, format!("material:{id}:none")));
        };
        let provenance = match resolved.source {
            AssignmentSource::Occurrence => "occurrence".to_owned(),
            AssignmentSource::Type(type_object) => format!("type:{type_object}"),
        };
        let mut locator = format!("material:{id}:{provenance}:{}", resolved.assignment.id());
        let not_layered = |kind: &str| MaterialLayerError::NotLayered {
            object: object.clone(),
            kind: kind.into(),
        };
        let (set, direction) = match resolved.material {
            ResolvedMaterialSelect::Definition(MaterialDefinition::LayerSet(set)) => (set, None),
            ResolvedMaterialSelect::Usage(MaterialUsageDefinition::LayerSet(usage)) => {
                locator = format!("{locator}:usage:{}", usage.id());
                let target = usage.layer_set_id().map_err(refusal)?;
                let ResolvedMaterialSelect::Definition(MaterialDefinition::LayerSet(set)) =
                    view.resolve_material_select(target).map_err(refusal)?
                else {
                    return Err(MaterialLayerError::Unreadable(format!(
                        "{} references {target}, not an IfcMaterialLayerSet",
                        usage.id()
                    )));
                };
                let axis = match usage.layer_set_direction().map_err(refusal)? {
                    LayerSetDirection::Axis1 => LayerAxis::First,
                    LayerSetDirection::Axis2 => LayerAxis::Second,
                    LayerSetDirection::Axis3 => LayerAxis::Third,
                };
                let sense = match usage.direction_sense().map_err(refusal)? {
                    DirectionSense::Positive => LayerSense::Positive,
                    DirectionSense::Negative => LayerSense::Negative,
                };
                (set, Some(LayerDirection { axis, sense }))
            }
            ResolvedMaterialSelect::Definition(MaterialDefinition::Material(_)) => {
                return Err(not_layered("material"));
            }
            ResolvedMaterialSelect::Definition(MaterialDefinition::ConstituentSet(_)) => {
                return Err(not_layered("constituent-set"));
            }
            ResolvedMaterialSelect::Definition(MaterialDefinition::ProfileSet(_))
            | ResolvedMaterialSelect::Usage(
                MaterialUsageDefinition::ProfileSet(_)
                | MaterialUsageDefinition::ProfileSetTapering(_),
            ) => return Err(not_layered("profile-set")),
            ResolvedMaterialSelect::List(_) => return Err(not_layered("list")),
            ResolvedMaterialSelect::Definition(
                MaterialDefinition::Layer(_)
                | MaterialDefinition::LayerWithOffsets(_)
                | MaterialDefinition::Constituent(_)
                | MaterialDefinition::Profile(_)
                | MaterialDefinition::ProfileWithOffsets(_),
            ) => {
                return Err(MaterialLayerError::Unreadable(format!(
                    "{id} is associated with a lone layer, constituent or profile, not a set"
                )));
            }
        };
        let locator = format!("{locator}:{}", set.id());
        Ok((self.read_set(view, set)?, direction, locator))
    }

    /// The layers of `set` in its order.
    fn read_set(
        &self,
        view: MaterialView<'_>,
        set: MaterialLayerSet<'_>,
    ) -> Result<Vec<MaterialLayer>, MaterialLayerError> {
        let mut layers = Vec::new();
        for id in set.layer_ids().map_err(refusal)? {
            macro_rules! read_layer {
                ($layer:expr) => {{
                    let layer = $layer;
                    let material = match layer.material_id().map_err(refusal)? {
                        Some(material) => Some(
                            Material::try_from_view(view, material)
                                .map_err(refusal)?
                                .name()
                                .map_err(refusal)?
                                .to_owned(),
                        ),
                        None => None,
                    };
                    MaterialLayer {
                        locator: id.to_string(),
                        material,
                        name: stated(layer.name())?.map(str::to_owned),
                        category: stated(layer.category())?.map(str::to_owned),
                        thickness: self.thickness(layer.thickness().map_err(refusal)?),
                        ventilated: match layer.is_ventilated().map_err(refusal)? {
                            Some(LogicalValue::True) => Some(true),
                            Some(LogicalValue::False) => Some(false),
                            Some(LogicalValue::Unknown) | None => None,
                        },
                        priority: stated(layer.priority())?,
                    }
                }};
            }
            layers.push(match view.resolve_material_select(id).map_err(refusal)? {
                ResolvedMaterialSelect::Definition(MaterialDefinition::Layer(layer)) => {
                    read_layer!(layer)
                }
                ResolvedMaterialSelect::Definition(MaterialDefinition::LayerWithOffsets(layer)) => {
                    read_layer!(layer)
                }
                _ => {
                    return Err(MaterialLayerError::Unreadable(format!(
                        "{} references {id}, not an IfcMaterialLayer",
                        set.id()
                    )));
                }
            });
        }
        Ok(layers)
    }
}

impl MaterialLayerService for IfcMaterialLayers {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn material_layers(&self, object: &ObjectId) -> Result<MaterialLayers, MaterialLayerError> {
        let id = self.entity(object)?;
        let (layers, direction, locator) = self.layers(object, id)?;
        let evidence = Evidence::exact(
            object.source.clone(),
            format!("ifc:{}:{locator}", self.snapshots[0].fingerprint()),
        );
        MaterialLayers::try_new(object.clone(), layers, direction, evidence)
    }
}

/// An optional attribute; one the file's release does not declare cannot be
/// stated, so it is absent like an unset one.
fn stated<T>(value: Result<Option<T>, MaterialError>) -> Result<Option<T>, MaterialLayerError> {
    match value {
        Err(MaterialError::NotInSchema { .. }) => Ok(None),
        value => value.map_err(refusal),
    }
}

/// Ambiguity is a conflict; anything malformed leaves the layers unread.
#[allow(clippy::needless_pass_by_value)] // Passed to `map_err` by name.
fn refusal(error: MaterialError) -> MaterialLayerError {
    match error {
        MaterialError::AmbiguousAssignment { .. } | MaterialError::AmbiguousType { .. } => {
            MaterialLayerError::Conflicting(error.to_string())
        }
        _ => MaterialLayerError::Unreadable(error.to_string()),
    }
}
