//! Source-neutral rule and classification vocabulary.
//!
//! This is the contract layer: **what must be true** ([`rule`]) and **what
//! counts as what** ([`classification`]). It depends on nothing but `serde`,
//! so it can never introduce a cycle and never leaks a source format.
//!
//! ## Why this lives in the engine (ADR 0002)
//!
//! This vocabulary was authored inside a vendor application, but nothing in it
//! is vendor-specific: a `RuleDefinition` describes units, enum domains,
//! localized labels, severities and applicability for *any* backend. Leaving it
//! in the application forced every consumer to depend on that application to
//! say what a rule is.
//!
//! Backend *bindings* -- lowering a catalog entry into a concrete vendor
//! document -- remain in the application that owns the format.
//!
//! ## The two halves
//!
//! - [`rule`] -- what must be true. A parameterized definition, an instance
//!   bound to values, and the package tree a backend compiles.
//! - [`classification`] -- which elements a rule applies to, and what they are.
//!   Layered: predicate (leaf test), expression (And/Or/Not), scheme (named,
//!   ordered, methoded).
//!
//! They are siblings in one crate rather than two because they share identical
//! dependencies (none) and identical consumers. A crate boundary should isolate
//! a dependency, not a topic -- and a rule's applicability *is* a classification
//! expression, so splitting them would need a third shared crate underneath.
#![forbid(unsafe_code)]
// This vocabulary is moved verbatim from the application that authored it
// (ADR 0002). The allows below cover shape characteristics of that existing
// code, not defects: spec structs are wide flag records by nature, and rule
// validation is one long exhaustive match. Behavioural pedantic lints stay on
// -- `cast_precision_loss` caught a real exactness bug in `ParamValue::as_f64`
// during the move, which was fixed rather than silenced.
#![allow(
    missing_docs,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::return_self_not_must_use,
    clippy::struct_excessive_bools,
    clippy::too_many_lines,
    clippy::trivially_copy_pass_by_ref,
    clippy::needless_pass_by_value,
    clippy::match_same_arms,
    clippy::wildcard_imports,
    clippy::match_wildcard_for_single_variants
)]

pub mod classification;
pub mod rule;

pub use classification::{
    Assignment, Expr, MatchMethod, Operator, Predicate, Scheme, SchemeRef, SchemeRegistry,
    SchemeRow, State, Subject,
};
pub use rule::applicability::{Applicability, Combine, CompareOp, Selector};
pub use rule::assertion::{AssertionSpec, IdentifierRef, Severity};
pub use rule::definition::{InstanceConstraint, RuleDefinition, RuleExample};
pub use rule::instance::{PackageMetadata, RuleFolder, RuleInstance, RuleSetPackage};
pub use rule::target::{Fidelity, Target};
pub use rule::text::LocalizedText;
