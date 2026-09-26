//! Batteries-included facade for the Axioval rule engine.
//!
//! Re-exports are grouped the way the workspace is: the source-neutral
//! contracts and engine are always present, and each source adapter appears
//! under a module named for the format or library it adapts, behind a feature
//! of the same name. Output sinks follow the same rule (`bcf`).
#![forbid(unsafe_code)]

pub use axioval_engine as engine;
pub use axioval_ir as ir;
pub use axioval_rules as rules;

#[cfg(feature = "axiolid")]
pub use axioval_axiolid as axiolid;
#[cfg(feature = "bcf")]
pub use axioval_bcf as bcf;
#[cfg(feature = "icdd")]
pub use axioval_icdd as icdd;
#[cfg(feature = "ifc")]
pub use axioval_ifc as ifc;

/// Builds the maintained trusted capability registry.
///
/// # Errors
///
/// Returns an error if two maintained capabilities declare the same stable ID.
pub fn default_registry() -> Result<engine::CapabilityRegistry, engine::EngineError> {
    rules::register_builtins(engine::CapabilityRegistry::new())
}
