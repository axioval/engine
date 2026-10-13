//! Whether each representation item of a product's body is glazing, as its
//! presentation styles and materials state it: what a geometry host needs
//! to measure the glazed area of a door's leaves (#205).
//!
//! An item is read as the surface transparency `styles.rs` reads: the
//! item's own surface style is authoritative, and an `IfcMappedItem` with a
//! surface style of its own styles everything it maps in, the outermost
//! such mapped item first (as `styles.rs` stops at it). A style
//! transparent at all (`Transparency` above 0) is glazing, an opaque one
//! is not; styles that state both decide nothing.
//!
//! An item with no surface style of its own, on itself or on a mapped item
//! above it, is drawn with the styles of its product's material: one
//! material whose styles agree decides it, and so do several materials
//! whose styles all agree. Materials that disagree (a frame and a glass
//! constituent), a material without styles, or no styled material at all
//! leave the item undecided: IFC does not say which material an item is
//! made of, and its shape is never read as its material.

use ifc_model::{EntityId, Model};

use crate::materials::Materials;
use crate::release::Release;
use crate::styles::{self, StyleIndex, Surface};

/// Whether one representation item is glazing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemGlazing {
    /// Styled as, or drawn with a material that is, transparent.
    Glazed,
    /// Styled as, or drawn with a material that is, opaque.
    Opaque,
    /// Its styles or materials cannot decide it: why, in plain words.
    Undecided(String),
}

/// Reads whether body items are glazing, over one model's styles and
/// materials, indexed once.
pub struct GlazingReader {
    release: Release,
    index: StyleIndex,
    materials: Materials,
}

impl std::fmt::Debug for GlazingReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlazingReader")
            .field("release", &self.release.label)
            .finish_non_exhaustive()
    }
}

/// What the surfaces of one style holder say.
fn classified(surfaces: &[Surface]) -> ItemGlazing {
    let transparent = surfaces.iter().filter(|s| s.transparency > 0.0).count();
    if transparent == surfaces.len() {
        ItemGlazing::Glazed
    } else if transparent == 0 {
        ItemGlazing::Opaque
    } else {
        ItemGlazing::Undecided("its surface styles state it both transparent and opaque".into())
    }
}

impl GlazingReader {
    /// A reader over `model`, of the one release its header declares.
    ///
    /// # Errors
    ///
    /// When the header declares no supported release, or the model's style
    /// bindings are malformed.
    pub fn new(model: &Model) -> Result<Self, String> {
        let schemas = model.header().schema.clone();
        let release = Release::from_header(&schemas)
            .ok_or_else(|| format!("the model declares no supported schema ({schemas:?})"))?;
        let index = styles::index(release.schema, model)?;
        Ok(Self {
            release,
            index,
            materials: Materials::new(release),
        })
    }

    /// Whether `item` of `product`'s body, reached through the mapped
    /// items `mapped_by` (outermost first, as `ifc-geometry`'s
    /// `BodyItem::mapped_by` lists them), is glazing.
    #[must_use]
    pub fn item_glazing(
        &self,
        model: &Model,
        product: EntityId,
        mapped_by: &[EntityId],
        item: EntityId,
    ) -> ItemGlazing {
        let schema = self.release.schema;
        for holder in mapped_by.iter().copied().chain(std::iter::once(item)) {
            match self.index.item_surfaces(schema, model, holder) {
                Ok(surfaces) if surfaces.is_empty() => {}
                Ok(surfaces) => return classified(&surfaces),
                Err(error) => {
                    return ItemGlazing::Undecided(format!("its style cannot be read: {error}"));
                }
            }
        }
        self.material_glazing(model, product)
    }

    /// What the product's materials say of an item without a style.
    fn material_glazing(&self, model: &Model, product: EntityId) -> ItemGlazing {
        let unstyled = || {
            ItemGlazing::Undecided(
                "it has no surface style of its own and its material states none".into(),
            )
        };
        if !self.index.styles_materials() {
            return unstyled();
        }
        let materials = match self.materials.material_ids(model, product) {
            Ok(materials) => materials,
            Err(error) => {
                return ItemGlazing::Undecided(format!("its material cannot be read: {error}"));
            }
        };
        if materials.is_empty() {
            return unstyled();
        }
        let schema = self.release.schema;
        let mut found: Option<ItemGlazing> = None;
        for material in &materials {
            let surfaces =
                match self
                    .index
                    .material_surfaces(schema, model, std::slice::from_ref(material))
                {
                    Ok(surfaces) if surfaces.is_empty() => {
                        return ItemGlazing::Undecided(format!(
                            "it has no surface style of its own and its material {material} states \
                         none"
                        ));
                    }
                    Ok(surfaces) => surfaces,
                    Err(error) => {
                        return ItemGlazing::Undecided(format!(
                            "the style of its material {material} cannot be read: {error}"
                        ));
                    }
                };
            let glazing = classified(&surfaces);
            if matches!(glazing, ItemGlazing::Undecided(_)) {
                return glazing;
            }
            match &found {
                Some(seen) if *seen != glazing => {
                    return ItemGlazing::Undecided(
                        "it has no surface style of its own, and its product's materials are \
                         transparent and opaque, so which it is made of is not stated"
                            .into(),
                    );
                }
                _ => found = Some(glazing),
            }
        }
        found.unwrap_or_else(unstyled)
    }
}
