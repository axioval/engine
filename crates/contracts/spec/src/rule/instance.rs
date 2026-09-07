//! Rule *instances* and *ruleset packages* — the authored, ready-to-compile
//! artifacts.
//!
//! A [`RuleInstance`] binds a [`crate::rule::definition::RuleDefinition`] (by id) to
//! concrete parameter values, a name, and an optional severity override. A
//! [`RuleSetPackage`] is the tree of folders + instances that a backend
//! compiles into one `.cset` / IDS document / `OpenBimRL` spec.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::rule::applicability::Applicability;
use crate::rule::assertion::Severity;
use crate::rule::param::ParamValue;
use crate::rule::text::LocalizedText;

/// One configured use of a rule definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleInstance {
    /// Stable backend-neutral identity. Empty legacy values are replaced by a
    /// deterministic package path during binding.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// The [`crate::rule::definition::RuleDefinition::id`] this instantiates.
    pub definition_id: String,
    /// Display name for this instance (source rule name / IDS spec name).
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub name: LocalizedText,
    /// Parameter values keyed by [`crate::rule::param::ParameterSpec::id`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, ParamValue>,
    /// Optional per-instance applicability override (replaces the definition's
    /// default when present).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applicability: Option<Applicability>,
    /// Optional severity override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<Severity>,
    /// Optional custom failure message.
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub message: LocalizedText,
}

impl RuleInstance {
    pub fn new(definition_id: impl Into<String>) -> Self {
        Self {
            id: String::new(),
            definition_id: definition_id.into(),
            name: LocalizedText::empty(),
            params: BTreeMap::new(),
            applicability: None,
            severity: None,
            message: LocalizedText::empty(),
        }
    }
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }
    pub fn with_name(mut self, name: impl Into<LocalizedText>) -> Self {
        self.name = name.into();
        self
    }
    pub fn param(mut self, id: impl Into<String>, value: ParamValue) -> Self {
        self.params.insert(id.into(), value);
        self
    }
    /// Effective parameter value, falling back to the definition default via
    /// [`crate::validate`] helpers when needed (this raw getter does not apply
    /// defaults — validation/compilation does).
    pub fn get(&self, id: &str) -> Option<&ParamValue> {
        self.params.get(id)
    }
}

/// A folder in the ruleset tree (sources may nest rulesets as models).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleFolder {
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub name: LocalizedText,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<RuleFolder>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<RuleInstance>,
}

impl RuleFolder {
    pub fn new(name: impl Into<LocalizedText>) -> Self {
        Self {
            name: name.into(),
            folders: Vec::new(),
            rules: Vec::new(),
        }
    }
    pub fn with_rule(mut self, r: RuleInstance) -> Self {
        self.rules.push(r);
        self
    }
    pub fn with_folder(mut self, f: RuleFolder) -> Self {
        self.folders.push(f);
        self
    }

    /// Depth-first iterator over every rule instance in this subtree.
    pub fn all_rules(&self) -> Vec<&RuleInstance> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }
    fn collect<'a>(&'a self, out: &mut Vec<&'a RuleInstance>) {
        out.extend(self.rules.iter());
        for f in &self.folders {
            f.collect(out);
        }
    }
}

/// Package-level metadata.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PackageMetadata {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created: String,
    /// Preferred locale for backends that emit a single label.
    #[serde(default = "default_locale")]
    pub locale: String,
}

fn default_locale() -> String {
    "en".to_string()
}

/// A complete ruleset ready for compilation to one or more targets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleSetPackage {
    /// Stable package id (also the default output file stem).
    pub id: String,
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub name: LocalizedText,
    /// The root folder tree.
    pub root: RuleFolder,
    #[serde(default)]
    pub metadata: PackageMetadata,
}

impl RuleSetPackage {
    pub fn new(id: impl Into<String>, name: impl Into<LocalizedText>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            root: RuleFolder::new(LocalizedText::empty()),
            metadata: PackageMetadata {
                locale: "en".into(),
                ..Default::default()
            },
        }
    }

    /// Every rule instance in the package, depth-first.
    pub fn all_rules(&self) -> Vec<&RuleInstance> {
        self.root.all_rules()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_walk_and_serde() {
        let pkg = {
            let mut p = RuleSetPackage::new("fire-qa", LocalizedText::en("Fire QA"));
            p.root = RuleFolder::new(LocalizedText::empty())
                .with_folder(
                    RuleFolder::new(LocalizedText::en("Fire")).with_rule(
                        RuleInstance::new("building.required_property")
                            .with_name(LocalizedText::en("Walls need FireRating"))
                            .param("pset", ParamValue::text("Pset_WallCommon"))
                            .param("name", ParamValue::text("FireRating")),
                    ),
                )
                .with_rule(RuleInstance::new("building.required_property"));
            p
        };
        assert_eq!(pkg.all_rules().len(), 2);
        let json = serde_json::to_string_pretty(&pkg).unwrap();
        let back: RuleSetPackage = serde_json::from_str(&json).unwrap();
        assert_eq!(pkg, back);
    }

    #[test]
    fn portable_instance_id_roundtrips_and_defaults_empty() {
        let identified = RuleInstance::new("space.area_range").with_id("space-area-01");
        let json = serde_json::to_string(&identified).unwrap();
        assert!(json.contains("space-area-01"));
        assert_eq!(
            serde_json::from_str::<RuleInstance>(&json).unwrap().id,
            "space-area-01"
        );

        let legacy = r#"{"definition_id":"space.area_range"}"#;
        assert!(
            serde_json::from_str::<RuleInstance>(legacy)
                .unwrap()
                .id
                .is_empty()
        );
    }
}
