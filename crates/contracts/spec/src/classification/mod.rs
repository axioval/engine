//! Which elements something applies to — and what they *are*.
//!
//! **Status: designed + typed (R11).** Lowering to `.classification` /
//! `.filter` is R12.
//!
//! # Filter is the primitive; classification is the abstraction
//!
//! Source formats express element selection two ways, and the corpus settles how
//! they relate: a `ClassificationDocument` **contains** `filters`, while a
//! `ClassAndPropertyFilter` contains no classification. The containment is
//! one-directional, so these are not siblings — a classification is a *named,
//! ordered dispatch over filter predicates*.
//!
//! That difference is why the abstraction pays. With raw filters, a project
//! where walls are modelled as generic named objects needs a new table row in
//! *every rule* that touches walls. With a [`scheme::Scheme`] the exception is
//! absorbed once, in one named artifact carrying provenance, and every rule
//! stays canonical — it asks for "Wall", not for
//! `IfcWall OR (IfcBuildingElementProxy AND Name~"Wall*")`.
//!
//! # The layers
//!
//! ```text
//! predicate  →  expr (And/Or/Not)  →  scheme (named, ordered, methoded)
//!     ↓                ↓                        ↓
//! .filter row     .filter tree           .classification
//! ```
//!
//! Rules reference a scheme **by name** ([`mod@reference`]), never inline it —
//! that indirection is the whole point.
//!
//! # 🚨 Two things a naive model loses
//!
//! - **`MatchMethod` is semantic, not cosmetic.** `FirstMatch` makes row order
//!   load-bearing; `AllMatching` lets one component carry several
//!   classifications. Dropping it silently changes results.
//! - **Formula rows** (a leading `=` over column references `A`–`Z`, `AA`–`ZZ`)
//!   are evaluated per component at check time. Flattening one to a literal
//!   name produces confidently wrong output.
//!
//! # Deliberately not source-shaped
//!
//! "A named group of elements defined by predicates" exists in every BIM tool —
//! `IfcClassification`/`IfcGroup`, Navisworks selection sets, Revit filters,
//! IDS `applicability`. A neutral scheme lowers to any of them. Vendor
//! specifics (a vendor class leaf, a vendor formula dialect) belong at the
//! lowering boundary in `codec`, never here.

pub mod expr;
pub mod predicate;
pub mod reference;
pub mod scheme;

pub use expr::Expr;
pub use predicate::{Operator, Predicate, State, Subject};
pub use reference::{ResolveError, SchemeRef, SchemeRegistry};
pub use scheme::{Assignment, MatchMethod, Provenance, Scheme, SchemeRow};
