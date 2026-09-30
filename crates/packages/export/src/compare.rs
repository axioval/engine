//! Rules compared by what decides them.
//!
//! Two rules check the same thing when they select the same capability with
//! the same parameters (defaults included) over the same applicability,
//! with the same gates and grading. Concepts are compared by the names they
//! bind to, not by their ids, and rule ids by their position, since a
//! rule's id and a concept's id are only names a package chose. Names,
//! descriptions, messages and tags are presentation and left out.
//!
//! Every profile judges exactness this way, so a difference found here is a
//! difference in a check's result, and the rule it is found in is
//! [refused](crate::LossKind::Refused), never degraded.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use axioval_ir::contract::{
    DefinitionPackage, ExternalName, ObjectTypeDefinition, PropertyDefinition,
    PropertySetDefinition, RuleDefinition, RuleInstance,
};
use serde_json::Value as Json;

use crate::Loss;

/// The definitions and concepts of every definition package, by id.
#[derive(Clone, Debug, Default)]
pub struct Catalog<'p> {
    /// Rule definitions.
    pub definitions: BTreeMap<&'p str, &'p RuleDefinition>,
    /// Object-type concepts.
    pub object_types: BTreeMap<&'p str, &'p ObjectTypeDefinition>,
    /// Property concepts.
    pub properties: BTreeMap<&'p str, &'p PropertyDefinition>,
    /// Property-set concepts.
    pub property_sets: BTreeMap<&'p str, &'p PropertySetDefinition>,
}

/// How rules are compared, beyond what every comparison leaves out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Comparison {
    /// Parameters, as `(capability, parameter)`, that only say how a
    /// finding reads, not what it finds, and are left out.
    pub presentation_parameters: BTreeSet<(String, String)>,
    /// Whether object-type names are compared ignoring ASCII case, for a
    /// type system whose class names are case-insensitive.
    pub case_insensitive_object_types: bool,
}

impl Comparison {
    /// Leaves `parameter` of `capability` out of the comparison.
    #[must_use]
    pub fn presentation_parameter(
        mut self,
        capability: impl Into<String>,
        parameter: impl Into<String>,
    ) -> Self {
        self.presentation_parameters
            .insert((capability.into(), parameter.into()));
        self
    }

    /// Compares object-type names ignoring ASCII case.
    #[must_use]
    pub fn case_insensitive_object_types(mut self) -> Self {
        self.case_insensitive_object_types = true;
        self
    }
}

/// A rule's definition is in none of the definition packages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownDefinition(pub String);

impl fmt::Display for UnknownDefinition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "definition {} is in no definition package", self.0)
    }
}

impl std::error::Error for UnknownDefinition {}

impl<'p> Catalog<'p> {
    /// The definitions and concepts of `packages`; a later package's entry
    /// replaces an earlier one's of the same id.
    #[must_use]
    pub fn new(packages: &'p [DefinitionPackage]) -> Self {
        let mut catalog = Self::default();
        for package in packages {
            for (id, definition) in &package.definitions {
                catalog.definitions.insert(id, definition);
            }
            for (id, concept) in &package.object_types {
                catalog.object_types.insert(id, concept);
            }
            for (id, concept) in &package.properties {
                catalog.properties.insert(id, concept);
            }
            for (id, concept) in &package.property_sets {
                catalog.property_sets.insert(id, concept);
            }
        }
        catalog
    }

    /// What decides `rules`, one JSON value per rule, with every concept
    /// replaced by the names it binds to and every rule id by its position:
    /// capability, parameters (with the definition's defaults),
    /// applicability, enablement, severity, gate, auxiliary flag, severity
    /// bands and overrides, and categories. Names, descriptions, messages,
    /// tags and `comparison`'s presentation parameters are left out.
    ///
    /// # Errors
    ///
    /// A rule whose definition is in no package.
    pub fn canonical(
        &self,
        rules: &[&RuleInstance],
        comparison: &Comparison,
    ) -> Result<Vec<Json>, UnknownDefinition> {
        let positions: BTreeMap<&str, usize> = rules
            .iter()
            .enumerate()
            .map(|(index, rule)| (rule.id.as_str(), index + 1))
            .collect();
        rules
            .iter()
            .map(|rule| {
                let definition = self
                    .definitions
                    .get(rule.definition_id.as_str())
                    .ok_or_else(|| UnknownDefinition(rule.definition_id.clone()))?;
                let mut parameters = rule.parameters.clone();
                for (id, parameter) in &definition.parameters {
                    if let Some(default) = &parameter.default_value {
                        parameters
                            .entry(id.clone())
                            .or_insert_with(|| default.clone());
                    }
                }
                for (capability, parameter) in &comparison.presentation_parameters {
                    if *capability == definition.capability {
                        // What a finding says, not what it finds.
                        parameters.remove(parameter);
                    }
                }
                let mut json = serde_json::json!({
                    "capability": definition.capability,
                    "parameters": parameters,
                    "applicability": rule.applicability,
                    "enabled": rule.enabled,
                    "severity": rule.severity,
                    "gate": rule.gate,
                    "auxiliary": rule.auxiliary,
                    "severityBands": rule.severity_bands,
                    "severityOverrides": rule.severity_overrides,
                    "categories": rule.categories,
                });
                self.substitute(&mut json, &positions, comparison);
                Ok(json)
            })
            .collect()
    }

    /// Replaces every concept id in `json` by what it binds to, and every
    /// rule id in `positions` by `rule #<position>`.
    pub fn substitute(
        &self,
        json: &mut Json,
        positions: &BTreeMap<&str, usize>,
        comparison: &Comparison,
    ) {
        match json {
            Json::String(text) => {
                if let Some(position) = positions.get(text.as_str()) {
                    *text = format!("rule #{position}");
                } else if let Some(token) = self.concept_token(text, comparison) {
                    *text = token;
                }
            }
            Json::Array(items) => {
                for item in items {
                    self.substitute(item, positions, comparison);
                }
            }
            Json::Object(fields) => {
                for value in fields.values_mut() {
                    self.substitute(value, positions, comparison);
                }
            }
            Json::Null | Json::Bool(_) | Json::Number(_) => {}
        }
    }

    /// The names the concept `id` binds to, sorted by type system; `None`
    /// when `id` is no concept.
    #[must_use]
    pub fn concept_token(&self, id: &str, comparison: &Comparison) -> Option<String> {
        let shown = |names: &[ExternalName], upper: bool| {
            let mut names: Vec<(String, String)> = names
                .iter()
                .map(|name| {
                    let text = if upper {
                        name.name.to_ascii_uppercase()
                    } else {
                        name.name.clone()
                    };
                    (name.type_system.clone(), text)
                })
                .collect();
            names.sort();
            format!("{names:?}")
        };
        let mut tokens = Vec::new();
        if let Some(concept) = self.object_types.get(id) {
            tokens.push(format!(
                "object type {}",
                shown(
                    &concept.external_names,
                    comparison.case_insensitive_object_types
                )
            ));
        }
        if let Some(concept) = self.properties.get(id) {
            tokens.push(format!(
                "property {}",
                shown(&concept.external_names, false)
            ));
        }
        if let Some(concept) = self.property_sets.get(id) {
            tokens.push(format!(
                "property set {}",
                shown(&concept.external_names, false)
            ));
        }
        (!tokens.is_empty()).then(|| tokens.join(" / "))
    }
}

/// Where `actual` rules, whose definitions and concepts are in `ours`,
/// check something other than `expected` rules, whose are in `theirs`:
/// the path to the first difference of their [canonical
/// forms](Catalog::canonical), rooted at `rules`, or `None` when they
/// decide alike.
///
/// # Errors
///
/// A rule whose definition is in no package; `expected` is read first.
pub fn verdict_difference(
    ours: &Catalog<'_>,
    actual: &[&RuleInstance],
    theirs: &Catalog<'_>,
    expected: &[&RuleInstance],
    comparison: &Comparison,
) -> Result<Option<String>, UnknownDefinition> {
    let expected = theirs.canonical(expected, comparison)?;
    let actual = ours.canonical(actual, comparison)?;
    Ok(first_difference(
        &Json::Array(actual),
        &Json::Array(expected),
        "rules",
    ))
}

/// The loss of the rule at `path` when [`verdict_difference`] finds a
/// difference: always [refused](crate::LossKind::Refused), since the
/// difference decides a check.
#[must_use]
pub fn difference_loss(path: impl Into<String>, difference: &str) -> Loss {
    Loss::refused(
        path,
        format!("the exported form checks something else: it differs at {difference}"),
    )
}

/// The path to the first place `actual` and `expected` differ, `None` when
/// they are equal. A field missing on one side equals `null` on the other.
#[must_use]
pub fn first_difference(actual: &Json, expected: &Json, path: &str) -> Option<String> {
    match (actual, expected) {
        (Json::Object(a), Json::Object(b)) => {
            let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
            keys.sort();
            keys.dedup();
            keys.into_iter().find_map(|key| {
                let inner = format!("{path}.{key}");
                match (a.get(key), b.get(key)) {
                    (Some(a), Some(b)) => first_difference(a, b, &inner),
                    (None, Some(Json::Null)) | (Some(Json::Null), None) => None,
                    _ => Some(inner),
                }
            })
        }
        (Json::Array(a), Json::Array(b)) if a.len() == b.len() => a
            .iter()
            .zip(b)
            .enumerate()
            .find_map(|(index, (a, b))| first_difference(a, b, &format!("{path}[{index}]"))),
        (Json::Array(a), Json::Array(b)) => Some(format!(
            "{path} ({} against {} as translated)",
            a.len(),
            b.len()
        )),
        _ if actual == expected => None,
        _ => Some(path.to_owned()),
    }
}
