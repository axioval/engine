//! Batteries-included facade for the Axioval rule engine.
//!
//! Re-exports are grouped the way the workspace is: the source-neutral
//! contracts and engine are always present, and each source adapter appears
//! under a module named for the format or library it adapts, behind a feature
//! of the same name. Output sinks follow the same rule (`bcf`, `bcf_api`
//! for BCF API servers, `bcf_snapshot` for viewpoint images, `xlsx` for
//! spreadsheet workbooks, and `html` for HTML reports).
#![forbid(unsafe_code)]

pub use axioval_engine as engine;
pub use axioval_ir as ir;
pub use axioval_rules as rules;

#[cfg(feature = "axiolid")]
pub use axioval_axiolid as axiolid;
#[cfg(feature = "bcf")]
pub use axioval_bcf as bcf;
#[cfg(feature = "bcf-api")]
pub use axioval_bcf_api as bcf_api;
#[cfg(feature = "bcf-snapshot")]
pub use axioval_bcf_snapshot as bcf_snapshot;
#[cfg(feature = "html")]
pub use axioval_html as html;
#[cfg(feature = "icdd")]
pub use axioval_icdd as icdd;
#[cfg(feature = "ifc")]
pub use axioval_ifc as ifc;
#[cfg(feature = "xlsx")]
pub use axioval_xlsx as xlsx;

/// Builds the maintained trusted capability registry.
///
/// # Errors
///
/// Returns an error if two maintained capabilities declare the same stable ID.
pub fn default_registry() -> Result<engine::CapabilityRegistry, engine::EngineError> {
    rules::register_builtins(engine::CapabilityRegistry::new())
}

/// The authoring catalogue of the default registry, with the concept
/// vocabulary of `packages`: what a rule may be built from, as versioned
/// JSON (see [`engine::catalogue`]).
///
/// # Errors
///
/// When the registry cannot be built or a capability lacks catalogue
/// texts.
pub fn catalogue(
    packages: &[ir::DefinitionPackage],
) -> Result<engine::catalogue::Catalogue, Box<dyn std::error::Error + Send + Sync>> {
    Ok(rules::catalogue::catalogue(&default_registry()?, packages)?)
}
