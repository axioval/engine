//! Rule *definitions* — the reusable, parameterized description of a check.
//!
//! A [`RuleDefinition`] is authored once (in a rule directory under
//! `sol-rules-std`) and instantiated many times with different parameters. It
//! is format-neutral: backends read it plus a [`crate::rule::instance::RuleInstance`]
//! to emit CSET/IDS/OpenBimRL.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::rule::applicability::Applicability;
use crate::rule::assertion::{AssertionSpec, Severity};
use crate::rule::param::{ParamValue, ParameterSpec};
use crate::rule::target::{Fidelity, Target};
use crate::rule::text::LocalizedText;

/// A worked example instance for docs / MCP prompting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleExample {
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub title: LocalizedText,
    /// Parameter values that make a valid instance of this definition.
    pub params: BTreeMap<String, ParamValue>,
}

/// A semantic relationship among parameters that cannot be expressed by one
/// [`ParameterSpec`] in isolation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstanceConstraint {
    /// At least one named boolean parameter must resolve to `true` after
    /// defaults are applied.
    AtLeastOneTrue { parameters: Vec<String> },
    /// The resolved numeric value of `lower` must not exceed `upper`. Integer
    /// values participate through the same widening used for float parameters.
    NumericLessOrEqual { lower: String, upper: String },
}

/// The canonical definition of one checking rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleDefinition {
    /// Stable dotted id, e.g. `"building.required_property"`.
    pub id: String,
    /// `SemVer` of this definition's schema/behaviour.
    pub version: String,
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub title: LocalizedText,
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub description: LocalizedText,
    /// Free-form domain tags (`"fire"`, `"accessibility"`, `"spaces"`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Default applicability; an instance may narrow/override it.
    pub applicability: Applicability,
    /// Declared parameters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<ParameterSpec>,
    /// Cross-parameter semantic invariants enforced by package validation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instance_constraints: Vec<InstanceConstraint>,
    /// The requirements this rule enforces.
    pub assertions: Vec<AssertionSpec>,
    /// Default severity for failures (an instance may override).
    #[serde(default)]
    pub default_severity: Severity,
    /// Per-target fidelity declaration. A missing entry means
    /// [`Fidelity::Unsupported`] by default (see [`RuleDefinition::fidelity`]).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub targets: BTreeMap<Target, Fidelity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<RuleExample>,
}

impl RuleDefinition {
    /// Look up a parameter spec by id.
    pub fn parameter(&self, id: &str) -> Option<&ParameterSpec> {
        self.parameters.iter().find(|p| p.id == id)
    }

    /// Declared fidelity for `target`, defaulting to `Unsupported` when the
    /// definition doesn't mention the target at all.
    pub fn fidelity(&self, target: Target) -> Fidelity {
        self.targets
            .get(&target)
            .cloned()
            .unwrap_or(Fidelity::Unsupported {
                reason: format!(
                    "{} does not declare support for {}",
                    self.id,
                    target.as_str()
                ),
            })
    }

    /// True when this definition can be compiled to `target` at all.
    pub fn supports(&self, target: Target) -> bool {
        self.fidelity(target).is_usable()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::applicability::Applicability;
    use crate::rule::assertion::AssertionSpec;

    fn sample() -> RuleDefinition {
        let mut targets = BTreeMap::new();
        targets.insert(Target::Cset, Fidelity::Full);
        targets.insert(Target::Ids, Fidelity::Full);
        RuleDefinition {
            id: "building.required_property".into(),
            version: "0.1.0".into(),
            title: LocalizedText::en("Required property"),
            description: LocalizedText::empty(),
            tags: vec!["properties".into()],
            applicability: Applicability::ifc_classes(["IfcBuildingElement"]),
            parameters: vec![],
            instance_constraints: Vec::new(),
            assertions: vec![AssertionSpec::PropertyExists {
                pset: "$pset".into(),
                name: "$name".into(),
            }],
            default_severity: Severity::Critical,
            targets,
            examples: vec![],
        }
    }

    #[test]
    fn fidelity_defaults_unsupported() {
        let d = sample();
        assert!(d.supports(Target::Cset));
        assert!(d.supports(Target::Ids));
        assert!(!d.supports(Target::OpenBimRl));
        assert!(matches!(
            d.fidelity(Target::OpenBimRl),
            Fidelity::Unsupported { .. }
        ));
    }

    #[test]
    fn serde_roundtrip() {
        let d = sample();
        let json = serde_json::to_string_pretty(&d).unwrap();
        let back: RuleDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }
}
