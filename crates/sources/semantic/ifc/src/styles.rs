//! Surface transparency of an object's body, read from its presentation styles.
//!
//! IFC styles geometry, not products: an `IfcStyledItem` binds presentation
//! styles to one representation item, and an `IfcPresentationLayerWithStyle`
//! to the items on its layer. A material carries styles of its own through an
//! `IfcMaterialDefinitionRepresentation`. Per item, the item's own styles are
//! authoritative, as IFC states: a unique direct `IfcStyledItem` wins over
//! layer styles (the cascade of `ifc-style`'s `resolve_item_style`), and only
//! an item with no surface style of its own is drawn with the styles of the
//! object's material. An `IfcMappedItem` without a surface style of its own
//! is transparent to the lookup: the items of the representation it maps in
//! are read instead.
//!
//! Only body representations are read (`Body`, `Body-FallBack`, or no
//! identifier): they are what can be seen and what can block a view. Only
//! surface styles count; curve, fill-area and text styles say nothing about a
//! surface. A surface style's transparency is its `IfcSurfaceStyleShading`
//! (or `IfcSurfaceStyleRendering`) `Transparency`, `0.0` (opaque) when unset
//! as the schema states; IFC2X3 shading has no such attribute and is opaque.
//! A surface style without a shading element states no transparency and is
//! refused, never guessed. Several direct `IfcStyledItem`s on one item are a
//! conflict.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::PropertyResolutionError;
use ifc_model::{EntityId, Model};
use ifc_schema::Schema;
use ifc_style::{StyleError, StyleView};

use crate::layers::{reference, references};

/// Holder -> (the styled item or layer assignment, its flattened styles).
type Bindings = BTreeMap<EntityId, Vec<(EntityId, Vec<EntityId>)>>;

/// Every style binding in one model, built once per session.
pub(crate) struct StyleIndex {
    /// Representation item -> direct `IfcStyledItem`s.
    direct: Bindings,
    /// Representation item -> `IfcPresentationLayerWithStyle`s with styles.
    layered: Bindings,
    /// `IfcMaterial` -> the styled items of its material representations.
    materials: Bindings,
}

/// One stated surface transparency and the surface style that states it.
pub(crate) struct Surface {
    pub(crate) transparency: f64,
    pub(crate) style: EntityId,
}

/// The surfaces of one object's body.
pub(crate) struct Surfaces {
    /// Surfaces styled on the object's own representation items.
    pub(crate) items: Vec<Surface>,
    /// Whether some body item carries no surface style of its own, so the
    /// material's styles draw it.
    pub(crate) unstyled: bool,
}

fn malformed(error: &StyleError) -> String {
    error.to_string()
}

/// Replaces IFC2X3 `IfcPresentationStyleAssignment` wrappers by their styles.
fn flatten(
    schema: &Schema,
    model: &Model,
    view: StyleView<'_, '_>,
    ids: Vec<EntityId>,
) -> Result<Vec<EntityId>, String> {
    let mut out = Vec::new();
    for id in ids {
        let entity = model
            .get(id)
            .ok_or_else(|| format!("{id} is referenced but absent"))?;
        if schema.is_a(&entity.type_name, "IfcPresentationStyleAssignment") {
            out.extend(
                view.presentation_style_assignment(id)
                    .and_then(|assignment| assignment.styles())
                    .map_err(|error| malformed(&error))?,
            );
        } else {
            out.push(id);
        }
    }
    Ok(out)
}

fn ids_of(schema: &Schema, model: &Model, root: &str) -> Vec<EntityId> {
    let mut types = vec![root];
    types.extend(schema.subtypes(root));
    let mut ids: Vec<EntityId> = types
        .into_iter()
        .flat_map(|type_name| model.ids_of_type(type_name).iter().copied())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Indexes every styled item, styled layer and styled material.
pub(crate) fn index(schema: &Schema, model: &Model) -> Result<StyleIndex, String> {
    let view = StyleView::new(model, schema);
    let mut direct = Bindings::new();
    let mut styled_items = BTreeMap::new();
    for id in ids_of(schema, model, "IfcStyledItem") {
        let binding = view.styled_item(id).map_err(|error| malformed(&error))?;
        let styles = flatten(
            schema,
            model,
            view,
            binding.styles().map_err(|error| malformed(&error))?,
        )?;
        if let Some(item) = binding.item().map_err(|error| malformed(&error))? {
            direct.entry(item).or_default().push((id, styles.clone()));
        }
        styled_items.insert(id, styles);
    }
    let mut layered = Bindings::new();
    for id in ids_of(schema, model, "IfcPresentationLayerAssignment") {
        let layer = view
            .presentation_layer(id)
            .map_err(|error| malformed(&error))?;
        let mut styles = flatten(
            schema,
            model,
            view,
            layer.layer_styles().map_err(|error| malformed(&error))?,
        )?;
        styles.sort_unstable();
        styles.dedup();
        if styles.is_empty() {
            continue;
        }
        for item in layer.assigned_items().map_err(|error| malformed(&error))? {
            layered.entry(item).or_default().push((id, styles.clone()));
        }
    }
    let mut materials = Bindings::new();
    for id in ids_of(schema, model, "IfcMaterialDefinitionRepresentation") {
        let Some(material) = reference(schema, model, id, "RepresentedMaterial")? else {
            return Err(format!("{id} represents no material"));
        };
        for representation in references(schema, model, id, "Representations")? {
            for item in references(schema, model, representation, "Items")? {
                let styles = styled_items.get(&item).ok_or_else(|| {
                    format!("{representation} styles {material} with {item}, not an IfcStyledItem")
                })?;
                materials
                    .entry(material)
                    .or_default()
                    .push((item, styles.clone()));
            }
        }
    }
    Ok(StyleIndex {
        direct,
        layered,
        materials,
    })
}

impl StyleIndex {
    /// Whether any material in the model carries styles.
    pub(crate) fn styles_materials(&self) -> bool {
        !self.materials.is_empty()
    }

    /// The styles that apply to `item`: its unique direct `IfcStyledItem`'s,
    /// else the union of its styled layers'.
    fn effective(&self, item: EntityId) -> Result<Vec<EntityId>, PropertyResolutionError> {
        match self.direct.get(&item).map(Vec::as_slice) {
            Some([(_, styles)]) => return Ok(styles.clone()),
            Some(several) if several.len() > 1 => {
                return Err(PropertyResolutionError::Conflicting(format!(
                    "{item} is styled by {} IfcStyledItems ({})",
                    several.len(),
                    several
                        .iter()
                        .map(|(id, _)| id.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            _ => {}
        }
        let mut styles: Vec<EntityId> = self
            .layered
            .get(&item)
            .into_iter()
            .flatten()
            .flat_map(|(_, styles)| styles.iter().copied())
            .collect();
        styles.sort_unstable();
        styles.dedup();
        Ok(styles)
    }

    /// The surfaces the styles of `materials` state, in material order.
    pub(crate) fn material_surfaces(
        &self,
        schema: &Schema,
        model: &Model,
        materials: &[EntityId],
    ) -> Result<Vec<Surface>, PropertyResolutionError> {
        let mut out = Vec::new();
        for material in materials {
            for (_, styles) in self.materials.get(material).into_iter().flatten() {
                out.extend(surfaces(schema, model, styles)?);
            }
        }
        Ok(out)
    }
}

/// The surfaces stated among `styles`; styles of other kinds are skipped.
fn surfaces(
    schema: &Schema,
    model: &Model,
    styles: &[EntityId],
) -> Result<Vec<Surface>, PropertyResolutionError> {
    let view = StyleView::new(model, schema);
    let refuse = |error: StyleError| PropertyResolutionError::Incomplete(error.to_string());
    let mut out = Vec::new();
    for &style in styles {
        let entity = model.get(style).ok_or_else(|| {
            PropertyResolutionError::Incomplete(format!("{style} is referenced but absent"))
        })?;
        if !schema.is_a(&entity.type_name, "IfcSurfaceStyle") {
            continue;
        }
        let elements = view
            .surface_style(style)
            .and_then(|surface| surface.elements())
            .map_err(refuse)?;
        let shading = elements.into_iter().find(|element| {
            model
                .get(*element)
                .is_some_and(|entity| schema.is_a(&entity.type_name, "IfcSurfaceStyleShading"))
        });
        let Some(shading) = shading else {
            return Err(PropertyResolutionError::Incomplete(format!(
                "surface style {style} has no IfcSurfaceStyleShading and states no transparency"
            )));
        };
        let transparency = view
            .surface_style_shading(shading)
            .and_then(|shading| shading.transparency())
            .map_err(refuse)?
            // Unset means opaque: the schema states 0.0 as the default.
            .unwrap_or(0.0);
        out.push(Surface {
            transparency,
            style,
        });
    }
    Ok(out)
}

/// The surfaces styled on the items of `object`'s body representations.
pub(crate) fn surfaces_of(
    schema: &Schema,
    model: &Model,
    index: &StyleIndex,
    object: EntityId,
) -> Result<Surfaces, PropertyResolutionError> {
    let incomplete = PropertyResolutionError::Incomplete;
    let mut found = Surfaces {
        items: Vec::new(),
        unstyled: false,
    };
    let Some(shape) = reference(schema, model, object, "Representation").map_err(incomplete)?
    else {
        return Ok(found);
    };
    let mut seen = BTreeSet::new();
    for representation in references(schema, model, shape, "Representations").map_err(incomplete)? {
        let is_shape = model
            .get(representation)
            .is_some_and(|entity| schema.is_a(&entity.type_name, "IfcShapeRepresentation"));
        if !is_shape || !is_body(schema, model, representation)? {
            continue;
        }
        visit(
            schema,
            model,
            index,
            representation,
            &mut found,
            &mut seen,
            0,
        )?;
    }
    Ok(found)
}

/// Whether `representation` is identified as a body, or not identified.
fn is_body(
    schema: &Schema,
    model: &Model,
    representation: EntityId,
) -> Result<bool, PropertyResolutionError> {
    let entity = model.get(representation).ok_or_else(|| {
        PropertyResolutionError::Incomplete(format!("{representation} is referenced but absent"))
    })?;
    let slot = schema
        .attribute_names(&entity.type_name)
        .iter()
        .position(|name| *name == "RepresentationIdentifier");
    match slot
        .and_then(|slot| entity.attribute(slot))
        .map(ifc_model::Value::unwrap_typed)
    {
        None | Some(ifc_model::Value::Null) => Ok(true),
        Some(ifc_model::Value::Text(identifier)) => Ok(identifier.eq_ignore_ascii_case("Body")
            || identifier.eq_ignore_ascii_case("Body-FallBack")),
        Some(_) => Err(PropertyResolutionError::Incomplete(format!(
            "{representation}.RepresentationIdentifier is not text"
        ))),
    }
}

fn visit(
    schema: &Schema,
    model: &Model,
    index: &StyleIndex,
    representation: EntityId,
    found: &mut Surfaces,
    seen: &mut BTreeSet<EntityId>,
    depth: usize,
) -> Result<(), PropertyResolutionError> {
    const MAX_DEPTH: usize = 8;
    let incomplete = PropertyResolutionError::Incomplete;
    if !seen.insert(representation) {
        return Ok(());
    }
    if depth > MAX_DEPTH {
        return Err(incomplete(format!(
            "{representation} maps geometry deeper than {MAX_DEPTH} levels"
        )));
    }
    for item in references(schema, model, representation, "Items").map_err(incomplete)? {
        let own = surfaces(schema, model, &index.effective(item)?)?;
        if !own.is_empty() {
            found.items.extend(own);
            continue;
        }
        let is_mapped = model
            .get(item)
            .is_some_and(|entity| schema.is_a(&entity.type_name, "IfcMappedItem"));
        let mapped = if is_mapped {
            reference(schema, model, item, "MappingSource")
                .map_err(incomplete)?
                .map(|map| reference(schema, model, map, "MappedRepresentation"))
                .transpose()
                .map_err(incomplete)?
                .flatten()
        } else {
            None
        };
        match mapped {
            Some(mapped) => visit(schema, model, index, mapped, found, seen, depth + 1)?,
            None => found.unstyled = true,
        }
    }
    Ok(())
}
