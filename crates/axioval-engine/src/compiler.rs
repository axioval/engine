//! Strict binding from portable schema packages to trusted executable plans.

use std::collections::{BTreeMap, BTreeSet};

use axioval_ir::contract::{ParameterKind, RuleFolder, RuleInstance};
use axioval_ir::{DefinitionPackage, RuleId, RuleSetPackage};

use crate::{
    CapabilityRegistry, CompiledRule, EngineError, ExecutionPlan, ParameterDescriptor,
    ParameterType,
};

/// Normalized Axioval Schema version implemented by this compiler.
pub const SUPPORTED_SCHEMA_VERSION: &str = "0.1.0";

/// Compiles a ruleset against its definition packages and host-controlled capabilities.
pub fn compile(
    registry: &CapabilityRegistry,
    definitions: &[DefinitionPackage],
    ruleset: &RuleSetPackage,
) -> Result<ExecutionPlan, EngineError> {
    validate_package_versions(definitions, ruleset)?;
    let packages = collect_definition_packages(definitions)?;
    for package_id in &ruleset.definition_packages {
        if !packages.contains_key(package_id.as_str()) {
            return Err(EngineError::MissingDefinitionPackage(package_id.clone()));
        }
    }
    let mut catalog = BTreeMap::new();
    for package_id in &ruleset.definition_packages {
        for (id, definition) in &packages[package_id.as_str()].definitions {
            if catalog.insert(id.as_str(), definition).is_some() {
                return Err(EngineError::CapabilityContract {
                    definition: id.clone(),
                    capability: definition.capability.clone(),
                    detail: "duplicate definition id".into(),
                });
            }
        }
    }
    let mut authored = Vec::new();
    flatten(&ruleset.root, &mut authored);
    authored.sort_by(|left, right| left.id.cmp(&right.id));
    let mut ids = BTreeSet::new();
    let mut rules = Vec::new();
    for rule in authored.into_iter().filter(|rule| rule.enabled) {
        if !ids.insert(rule.id.as_str()) {
            return Err(EngineError::DuplicateRule(rule.id.clone()));
        }
        let definition = catalog
            .get(rule.definition_id.as_str())
            .ok_or_else(|| EngineError::UnknownDefinition(rule.definition_id.clone()))?;
        let capability = registry
            .get(&definition.capability)
            .ok_or_else(|| EngineError::UnknownCapability(definition.capability.clone()))?;
        let descriptors = capability.parameters();
        validate_signature(
            &rule.definition_id,
            &definition.capability,
            &descriptors,
            &definition.parameters,
        )?;
        let mut parameters = rule.parameters.clone();
        for (name, parameter) in &definition.parameters {
            if !parameters.contains_key(name) {
                if let Some(default) = &parameter.default_value {
                    parameters.insert(name.clone(), default.clone());
                } else if parameter.required {
                    return Err(EngineError::MissingParameter {
                        capability: definition.capability.clone(),
                        parameter: name.clone(),
                    });
                }
            }
        }
        let known: BTreeMap<_, _> = descriptors
            .iter()
            .map(|item| (item.name.as_str(), item))
            .collect();
        for (name, value) in &parameters {
            let descriptor =
                known
                    .get(name.as_str())
                    .ok_or_else(|| EngineError::UnknownParameter {
                        capability: definition.capability.clone(),
                        parameter: name.clone(),
                    })?;
            if !descriptor.parameter_type.accepts(value) {
                return Err(EngineError::InvalidParameterType {
                    capability: definition.capability.clone(),
                    parameter: name.clone(),
                });
            }
            let definition_parameter = &definition.parameters[name];
            if !definition_parameter.allowed_values.is_empty()
                && !definition_parameter.allowed_values.contains(value)
            {
                return Err(EngineError::CapabilityContract {
                    definition: rule.definition_id.clone(),
                    capability: definition.capability.clone(),
                    detail: format!("parameter `{name}` is outside allowedValues"),
                });
            }
        }
        rules.push(CompiledRule {
            id: RuleId::new(rule.id.clone())
                .map_err(|_| EngineError::InvalidRuleId(rule.id.clone()))?,
            capability: definition.capability.clone(),
            severity: rule.severity.clone(),
            selector: rule.applicability.clone(),
            parameters,
        });
    }
    Ok(ExecutionPlan { rules })
}

fn collect_definition_packages(
    definitions: &[DefinitionPackage],
) -> Result<BTreeMap<&str, &DefinitionPackage>, EngineError> {
    let mut packages = BTreeMap::new();
    for package in definitions {
        if packages
            .insert(package.package.id.as_str(), package)
            .is_some()
        {
            return Err(EngineError::DuplicateDefinitionPackage(
                package.package.id.clone(),
            ));
        }
    }
    Ok(packages)
}

fn validate_package_versions(
    definitions: &[DefinitionPackage],
    ruleset: &RuleSetPackage,
) -> Result<(), EngineError> {
    validate_schema_version(
        "ruleset package",
        &ruleset.package.id,
        &ruleset.schema_version,
    )?;
    for package in definitions {
        validate_schema_version(
            "definition package",
            &package.package.id,
            &package.schema_version,
        )?;
    }
    Ok(())
}

fn validate_schema_version(
    package_kind: &'static str,
    package_id: &str,
    version: &str,
) -> Result<(), EngineError> {
    if version == SUPPORTED_SCHEMA_VERSION {
        return Ok(());
    }
    Err(EngineError::UnsupportedSchemaVersion {
        package_kind,
        package_id: package_id.into(),
        version: version.into(),
        supported: SUPPORTED_SCHEMA_VERSION,
    })
}

fn flatten<'a>(folder: &'a RuleFolder, out: &mut Vec<&'a RuleInstance>) {
    out.extend(&folder.rules);
    for child in &folder.folders {
        flatten(child, out);
    }
}

fn validate_signature(
    definition_id: &str,
    capability_id: &str,
    descriptors: &[ParameterDescriptor],
    parameters: &BTreeMap<String, axioval_ir::contract::ParameterDefinition>,
) -> Result<(), EngineError> {
    if descriptors.len() != parameters.len() {
        return contract_error(definition_id, capability_id, "parameter count differs");
    }
    for descriptor in descriptors {
        let Some(parameter) = parameters.get(&descriptor.name) else {
            return contract_error(definition_id, capability_id, "parameter name differs");
        };
        if descriptor.required != parameter.required
            || descriptor.parameter_type != from_kind(&parameter.kind)
        {
            return contract_error(definition_id, capability_id, "parameter signature differs");
        }
    }
    Ok(())
}

fn contract_error<T>(definition: &str, capability: &str, detail: &str) -> Result<T, EngineError> {
    Err(EngineError::CapabilityContract {
        definition: definition.into(),
        capability: capability.into(),
        detail: detail.into(),
    })
}

fn from_kind(kind: &ParameterKind) -> ParameterType {
    match kind {
        ParameterKind::String => ParameterType::String,
        ParameterKind::Boolean => ParameterType::Boolean,
        ParameterKind::Integer => ParameterType::Integer,
        ParameterKind::Number => ParameterType::Number,
        ParameterKind::Quantity => ParameterType::Quantity,
        ParameterKind::Enum => ParameterType::Enum,
        ParameterKind::Reference => ParameterType::Reference,
        ParameterKind::ObjectTypeReference => ParameterType::ObjectTypeReference,
        ParameterKind::PropertyReference => ParameterType::PropertyReference,
        ParameterKind::Selector => ParameterType::Selector,
        ParameterKind::StringList => ParameterType::StringList,
        ParameterKind::ReferenceList => ParameterType::ReferenceList,
    }
}
