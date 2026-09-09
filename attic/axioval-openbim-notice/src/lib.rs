//! **This crate is deprecated. Use [`axioval-ifc`](https://crates.io/crates/axioval-ifc).**
//!
//! `axioval-openbim` was renamed to `axioval-ifc` in Axioval 0.1.12. The name
//! change reflects what the adapter actually maps: IFC semantics, rather than
//! the broader OpenBIM umbrella. Nothing was removed in the rename.
//!
//! # Migrating
//!
//! ```toml
//! # before
//! axioval-openbim = "0.1"
//!
//! # after
//! axioval-ifc = "0.1"
//! ```
//!
//! Rust paths change from `axioval_openbim::` to `axioval_ifc::`. If you use
//! the `axioval` facade, the feature flag `openbim` is now `ifc`. The types,
//! their methods and their behaviour are otherwise unchanged.
//!
//! # Why this release exists
//!
//! crates.io has no rename primitive, so `axioval-ifc` was published as a new
//! crate and this name was left behind pointing at nothing. This release
//! replaces the last functional version with a notice so the name leads
//! somewhere instead of dead-ending at code that no longer receives fixes.
//!
//! The final functional release under this name is `0.1.11`. It is intentionally
//! **not** yanked: yanking would break the lockfile of anyone already building
//! against it, while telling them nothing about where the code went. Pinning
//! `=0.1.11` keeps working; it simply will not receive updates.
//!
//! This is a `0.2.0` rather than a `0.1.12` on purpose. Under Cargo's semver
//! rules a `0.1.12` would be picked up automatically by anyone depending on
//! `"0.1"`, turning a routine update into an empty crate. A minor bump inside
//! `0.x` is a breaking change, so existing builds stay on `0.1.11` until the
//! author opts in.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

/// Marker for the rename, kept so the deprecation is visible from code and not
/// only from the README.
///
/// Referencing this item produces a deprecation warning naming the new crate.
/// It carries no data and has no behaviour.
#[deprecated(
    since = "0.2.0",
    note = "axioval-openbim was renamed to axioval-ifc; depend on `axioval-ifc` instead"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Renamed;

/// The crate that replaced this one.
pub const REPLACEMENT: &str = "axioval-ifc";

/// The last release of this crate that contained a working adapter.
pub const FINAL_FUNCTIONAL_VERSION: &str = "0.1.11";

#[cfg(test)]
mod tests {
    use super::{FINAL_FUNCTIONAL_VERSION, REPLACEMENT};

    /// The notice must name where the code went; a deprecation that does not
    /// say what replaced it leaves the reader exactly where they started.
    #[test]
    fn the_notice_names_its_replacement_and_final_version() {
        assert_eq!(REPLACEMENT, "axioval-ifc");
        assert_eq!(FINAL_FUNCTIONAL_VERSION, "0.1.11");
    }
}
