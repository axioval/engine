//! **This crate is retired.** Rule packages are
//! [`axioval-ir`](https://crates.io/crates/axioval-ir); writing them as other
//! formats is [`axioval-export`](https://crates.io/crates/axioval-export).
//!
//! No Axioval crate depended on `axioval-spec`. Its types duplicated what
//! the package contracts in `axioval-ir` state, and its export targets were a
//! closed list shaped after particular host applications, which a
//! source-neutral engine cannot keep.
//!
//! # Migrating
//!
//! - Rule definitions, rule instances, rulesets, selectors and severities
//!   are in `axioval_ir::contract`.
//! - To write a ruleset as another format, implement
//!   `axioval_export::ExportProfile`: a profile has an open string id and
//!   reports every loss as refused (the item's checking meaning cannot be
//!   expressed) or degraded (structure or presentation only).
//!
//! # Versions
//!
//! The last release with code is `0.3.0`; it is not yanked, so existing
//! lockfiles keep building. This notice is `0.4.0`, so no build depending on
//! `"0.3"` picks it up by accident.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

/// Marker for the retirement, so it shows in code and not only in the
/// README. Referencing it warns with the crates that replace this one.
#[deprecated(
    since = "0.4.0",
    note = "axioval-spec is retired; use `axioval-ir` for rule packages and `axioval-export` for export targets"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retired;

/// The crates that replace this one.
pub const REPLACEMENTS: [&str; 2] = ["axioval-ir", "axioval-export"];

/// The last release of this crate that contained code.
pub const FINAL_FUNCTIONAL_VERSION: &str = "0.3.0";

#[cfg(test)]
mod tests {
    use super::{FINAL_FUNCTIONAL_VERSION, REPLACEMENTS};

    /// A retirement notice must say where to go instead.
    #[test]
    fn the_notice_names_its_replacements_and_final_version() {
        assert_eq!(REPLACEMENTS, ["axioval-ir", "axioval-export"]);
        assert_eq!(FINAL_FUNCTIONAL_VERSION, "0.3.0");
    }
}
