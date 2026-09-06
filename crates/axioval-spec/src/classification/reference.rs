//! Referencing a [`Scheme`] **by name** — the indirection that is the point.
//!
//! A rule says "applies to `Wall`", not "applies to
//! `IfcWall OR (IfcBuildingElementProxy AND Name~"Wall*")`". The mapping from
//! the canonical concept to the messy model reality lives in one named scheme,
//! so a project-specific exception is absorbed once instead of being copied
//! into every rule that touches walls.
//!
//! Inlining a scheme at the use site would discard exactly that, which is why
//! there is no `Expr` variant that embeds a `Scheme`.
//!
//! [`Scheme`]: crate::classification::scheme::Scheme

use serde::{Deserialize, Serialize};

use super::scheme::Scheme;

/// A by-name pointer at a scheme, optionally narrowed to one of its classes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemeRef {
    /// The [`Scheme::id`] being referenced.
    pub scheme: String,
    /// A specific class within that scheme. `None` means "classified by this
    /// scheme at all", which is a genuinely different question from "classified
    /// as this particular class".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
}

impl SchemeRef {
    pub fn new(scheme: impl Into<String>) -> Self {
        SchemeRef {
            scheme: scheme.into(),
            class: None,
        }
    }

    pub fn class(scheme: impl Into<String>, class: impl Into<String>) -> Self {
        SchemeRef {
            scheme: scheme.into(),
            class: Some(class.into()),
        }
    }
}

/// Why a [`SchemeRef`] could not be resolved.
///
/// Distinguishing these is not pedantry: a missing scheme is an authoring error
/// the user must fix, while a missing class is often a typo against a scheme
/// that *is* present — and the fix differs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    #[error("no scheme named `{0}` is registered")]
    UnknownScheme(String),
    #[error("scheme `{scheme}` has no class named `{class}` (known: {known:?})")]
    UnknownClass {
        scheme: String,
        class: String,
        known: Vec<String>,
    },
}

/// A set of schemes available for resolution.
///
/// `BTreeMap`-backed so iteration order is deterministic — the same concern
/// that produced the model's hash-order bug.
#[derive(Debug, Clone, Default)]
pub struct SchemeRegistry {
    by_id: std::collections::BTreeMap<String, Scheme>,
}

impl SchemeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, scheme: Scheme) -> Option<Scheme> {
        self.by_id.insert(scheme.id.clone(), scheme)
    }

    pub fn get(&self, id: &str) -> Option<&Scheme> {
        self.by_id.get(id)
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// All schemes in id order.
    pub fn all(&self) -> impl Iterator<Item = &Scheme> {
        self.by_id.values()
    }

    /// Resolve a reference, checking the class exists when one is named.
    pub fn resolve(&self, r: &SchemeRef) -> Result<&Scheme, ResolveError> {
        let scheme = self
            .by_id
            .get(&r.scheme)
            .ok_or_else(|| ResolveError::UnknownScheme(r.scheme.clone()))?;

        if let Some(class) = &r.class {
            if !scheme.class_names().contains(&class.as_str()) {
                return Err(ResolveError::UnknownClass {
                    scheme: r.scheme.clone(),
                    class: class.clone(),
                    known: scheme
                        .class_names()
                        .into_iter()
                        .map(str::to_string)
                        .collect(),
                });
            }
        }
        Ok(scheme)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classification::expr::Expr;
    use crate::classification::predicate::Predicate;
    use crate::classification::scheme::{Assignment, MatchMethod, SchemeRow};

    fn registry() -> SchemeRegistry {
        let mut r = SchemeRegistry::new();
        r.insert(
            Scheme::new("building", MatchMethod::FirstMatch).with_row(SchemeRow {
                condition: Expr::leaf(Predicate::component_class("Wall")),
                assigns: Assignment::Name {
                    value: "Wall".into(),
                },
                material: None,
            }),
        );
        r
    }

    #[test]
    fn resolves_a_known_scheme_and_class() {
        let r = registry();
        assert!(r.resolve(&SchemeRef::new("building")).is_ok());
        assert!(r.resolve(&SchemeRef::class("building", "Wall")).is_ok());
    }

    #[test]
    fn unknown_scheme_and_unknown_class_are_different_errors() {
        let r = registry();
        assert!(matches!(
            r.resolve(&SchemeRef::new("nope")),
            Err(ResolveError::UnknownScheme(_))
        ));
        // A typo against a real scheme must not report "unknown scheme".
        assert!(matches!(
            r.resolve(&SchemeRef::class("building", "Wal")),
            Err(ResolveError::UnknownClass { .. })
        ));
    }

    #[test]
    fn unknown_class_error_lists_what_was_available() {
        let r = registry();
        match r.resolve(&SchemeRef::class("building", "Wal")) {
            Err(ResolveError::UnknownClass { known, .. }) => {
                assert_eq!(known, vec!["Wall".to_string()]);
            }
            other => panic!("expected UnknownClass, got {other:?}"),
        }
    }
}
