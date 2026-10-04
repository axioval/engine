//! The authoring catalogue of the built-in capabilities: the engine's
//! catalogue with each capability's labels and help from
//! `catalogue_texts.rs`.

use axioval_engine::CapabilityRegistry;
use axioval_engine::catalogue::{CapabilityTexts, Catalogue, CatalogueError};
use axioval_ir::DefinitionPackage;

use crate::catalogue_texts::CAPABILITY_TEXTS;

/// The texts of the built-in capability `id`.
fn texts(id: &str) -> Option<CapabilityTexts> {
    CAPABILITY_TEXTS
        .binary_search_by(|text| text.id.cmp(id))
        .ok()
        .map(|index| {
            let text: &'static _ = &CAPABILITY_TEXTS[index];
            CapabilityTexts {
                label: &text.label,
                help: &text.help,
                parameters: text.parameters,
            }
        })
}

/// The authoring catalogue of what `registry` runs, described by the
/// built-in capabilities' texts, with the concept vocabulary of
/// `packages`.
///
/// # Errors
///
/// [`CatalogueError`] when `registry` holds a capability that is not
/// built in, or a built-in one, a parameter or a measured value lacks
/// texts in English and German.
pub fn catalogue(
    registry: &CapabilityRegistry,
    packages: &[DefinitionPackage],
) -> Result<Catalogue, CatalogueError> {
    axioval_engine::catalogue::catalogue(registry, texts, packages)
}
