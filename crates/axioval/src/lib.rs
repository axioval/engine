//! Batteries-included facade for the Axioval rule engine.
#![forbid(unsafe_code)]

pub use axioval_engine as engine;
pub use axioval_ir as ir;
pub use axioval_rules as rules;

#[cfg(feature = "axiolid")]
pub use axioval_axiolid as axiolid;
#[cfg(feature = "icdd")]
pub use axioval_icdd as icdd;
#[cfg(feature = "openbim")]
pub use axioval_openbim as openbim;

/// Builds the maintained trusted capability registry.
///
/// # Errors
///
/// Returns an error if two maintained capabilities declare the same stable ID.
pub fn default_registry() -> Result<engine::CapabilityRegistry, engine::EngineError> {
    rules::register_builtins(engine::CapabilityRegistry::new())
}
