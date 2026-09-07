//! Applicability — *which* model elements a rule inspects.
//!
//! This is the format-neutral selector model. One source format expresses it as
//! `ClassAndPropertyFilter` chains; IDS as `<applicability>` facets; `OpenBimRL`
//! as node/predicate selectors. The IR keeps a small, composable selector set
//! that all three can lower from.

use serde::{Deserialize, Serialize};

/// Comparison operator for property/attribute value selectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareOp {
    Equal,
    NotEqual,
    GreaterThan,
    GreaterOrEqual,
    LessThan,
    LessOrEqual,
    /// Substring / contains match on the string form.
    Contains,
    /// Regular-expression match on the string form.
    Matches,
}

/// A single selector predicate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "selector", rename_all = "snake_case")]
pub enum Selector {
    /// Element `is_a` one of these IFC entity types (recursive subtype match).
    /// A parameter reference (`$param_id`) may be used in place of a literal
    /// list; the compiler resolves it against the instance's params.
    IfcClass { any_of: Vec<String> },
    /// Element carries a classification under `system` (optionally with a code
    /// in `any_code`).
    Classification {
        system: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        any_code: Vec<String>,
    },
    /// Element has a property `name` in property-set `pset`, optionally
    /// compared against `value` with `op`.
    Property {
        pset: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        op: Option<CompareOp>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<String>,
    },
    /// Element's IFC type/predefined-type attribute equals one of these.
    PredefinedType { any_of: Vec<String> },
}

/// How child selectors combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Combine {
    /// All predicates must hold.
    All,
    /// At least one predicate must hold.
    Any,
}

/// The full applicability scope of a rule: a combination of selectors plus an
/// optional exclusion set (elements matching `exclude` are dropped).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Applicability {
    #[serde(default = "default_combine")]
    pub combine: Combine,
    pub include: Vec<Selector>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<Selector>,
}

fn default_combine() -> Combine {
    Combine::All
}

impl Default for Applicability {
    fn default() -> Self {
        Self {
            combine: Combine::All,
            include: Vec::new(),
            exclude: Vec::new(),
        }
    }
}

impl Applicability {
    /// Applicability that selects a single set of IFC classes.
    pub fn ifc_classes<I, S>(classes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            combine: Combine::All,
            include: vec![Selector::IfcClass {
                any_of: classes.into_iter().map(Into::into).collect(),
            }],
            exclude: Vec::new(),
        }
    }

    pub fn and(mut self, s: Selector) -> Self {
        self.include.push(s);
        self
    }

    pub fn excluding(mut self, s: Selector) -> Self {
        self.exclude.push(s);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_and_serde() {
        let a = Applicability::ifc_classes(["IfcWall", "IfcSlab"]).and(Selector::Property {
            pset: "Pset_WallCommon".into(),
            name: "IsExternal".into(),
            op: Some(CompareOp::Equal),
            value: Some("true".into()),
        });
        let json = serde_json::to_string(&a).unwrap();
        let back: Applicability = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
        assert_eq!(a.include.len(), 2);
    }

    #[test]
    fn default_combine_is_all() {
        let json = r#"{"include":[{"selector":"ifc_class","any_of":["IfcDoor"]}]}"#;
        let a: Applicability = serde_json::from_str(json).unwrap();
        assert_eq!(a.combine, Combine::All);
    }
}
