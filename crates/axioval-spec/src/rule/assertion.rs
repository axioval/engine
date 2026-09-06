//! Assertions — *what must be true* of the applicable elements.
//!
//! Applicability selects the element set; assertions are the requirement
//! checked against each. Kept format-neutral: IDS lowers these to
//! `<requirements>`, CSET to rule-logic parameters, `OpenBimRL` to predicates.
//!
//! Parameter references: string fields may contain `$param_id`, resolved by a
//! backend against the rule instance's parameter values. This keeps a single
//! definition reusable across many instances.

use serde::{Deserialize, Serialize};

use crate::rule::applicability::CompareOp;

/// The severity a failed assertion produces.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    #[default]
    Critical,
}

/// A single requirement checked against each applicable element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "assert", rename_all = "snake_case")]
pub enum AssertionSpec {
    /// A property `name` in `pset` must exist (and be non-empty).
    PropertyExists { pset: String, name: String },
    /// A property must satisfy `op value`.
    PropertyValue {
        pset: String,
        name: String,
        op: CompareOp,
        value: String,
    },
    /// A property's value must be one of `allowed`.
    PropertyInSet {
        pset: String,
        name: String,
        allowed: Vec<String>,
    },
    /// A numeric property must lie in the inclusive range `[min, max]`.
    NumericRange {
        pset: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
    },
    /// Element must carry a classification under `system`.
    ClassificationRequired {
        system: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        any_code: Vec<String>,
    },
    /// The identifier's value must be **unique across every applicable
    /// element**, not merely present on each one.
    ///
    /// 🚨 This is the first assertion whose truth is not decidable per element:
    /// every other variant here can be evaluated by looking at one element,
    /// whereas uniqueness needs the whole applicable set. A backend that
    /// evaluates assertions element-by-element must special-case it rather than
    /// silently reporting "passes" for each element in isolation.
    ///
    /// `source` names *which* value is the identifier — an IFC attribute
    /// (`name`, `object_type`) or a property — because the provider's
    /// `UniqueSpaceNumberConstraint` lets the author choose, and most IFC
    /// exports write a space's "Number" into its `Name` attribute.
    ValueUnique {
        source: IdentifierRef,
        /// Whether a missing or blank value is itself a problem. the provider reports
        /// it as the separate "no identifier" issue, so it is a distinct flag
        /// rather than being implied by uniqueness.
        #[serde(default)]
        require_present: bool,
    },
    /// A whole-model spatial/geometric requirement handled by a named backend
    /// capability rather than a per-element property check. `config` is an
    /// opaque JSON blob interpreted by the rule's mapper (e.g. a clash matrix).
    BackendCapability {
        capability: String,
        #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
        config: serde_json::Value,
    },
}

/// Which value on an element acts as its identifier.
///
/// Kept separate from [`AssertionSpec`] so other assertions can adopt it later
/// (e.g. a future "values must match a pattern") without reshaping this enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "from", rename_all = "snake_case")]
pub enum IdentifierRef {
    /// The element's IFC `Name` attribute.
    Name,
    /// The element's IFC `ObjectType` attribute.
    ObjectType,
    /// A property, optionally scoped to a property set. `pset: None` means
    /// "any property set carrying this property name".
    Property {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pset: Option<String>,
        name: String,
    },
}

impl AssertionSpec {
    /// A short discriminant string, handy for diagnostics and coverage tables.
    pub fn kind(&self) -> &'static str {
        match self {
            AssertionSpec::PropertyExists { .. } => "property_exists",
            AssertionSpec::PropertyValue { .. } => "property_value",
            AssertionSpec::PropertyInSet { .. } => "property_in_set",
            AssertionSpec::NumericRange { .. } => "numeric_range",
            AssertionSpec::ClassificationRequired { .. } => "classification_required",
            AssertionSpec::ValueUnique { .. } => "value_unique",
            AssertionSpec::BackendCapability { .. } => "backend_capability",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_serde() {
        let a = AssertionSpec::PropertyExists {
            pset: "Pset_WallCommon".into(),
            name: "FireRating".into(),
        };
        assert_eq!(a.kind(), "property_exists");
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"assert\":\"property_exists\""));
        let back: AssertionSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn backend_capability_opaque_config() {
        let a = AssertionSpec::BackendCapability {
            capability: "clash.hard".into(),
            config: serde_json::json!({"tolerance_mm": 1.0}),
        };
        let json = serde_json::to_string(&a).unwrap();
        let back: AssertionSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
    }
}
