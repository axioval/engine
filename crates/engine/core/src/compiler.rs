//! Strict binding from portable schema packages to trusted executable plans.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use axioval_ir::contract::{
    ParameterKind, ParameterValue, RuleApplicability, RuleDefinition, RuleFolder, RuleInstance,
    Selector,
};
use axioval_ir::{DefinitionPackage, RuleId, RuleSetPackage};

use crate::concepts::{ConceptCatalog, ConceptKind};
use crate::{
    CapabilityRegistry, CompiledRule, DeferredRule, EngineError, ExecutionPlan,
    ParameterDescriptor, ParameterType,
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
    let catalog = definition_catalog(ruleset, &packages)?;
    let concepts = concept_catalog(ruleset, &packages)?;
    let mut authored = Vec::new();
    flatten(&ruleset.root, &mut authored);
    authored.sort_by(|left, right| left.id.cmp(&right.id));
    let mut ids = BTreeSet::new();
    let mut rules = Vec::new();
    let mut deferred = Vec::new();
    for rule in authored.into_iter().filter(|rule| rule.enabled) {
        if !ids.insert(rule.id.as_str()) {
            return Err(EngineError::DuplicateRule(rule.id.clone()));
        }
        let definition = catalog
            .get(rule.definition_id.as_str())
            .ok_or_else(|| EngineError::UnknownDefinition(rule.definition_id.clone()))?;
        let parameters = bind_parameters(registry, rule, definition)?;
        for value in parameters.values() {
            validate_parameter_concepts(&concepts, &rule.id, value)?;
        }
        let id = RuleId::new(rule.id.clone())
            .map_err(|_| EngineError::InvalidRuleId(rule.id.clone()))?;
        match applicability_selector(&concepts, rule)? {
            Ok(selector) => rules.push(CompiledRule {
                id,
                capability: definition.capability.clone(),
                severity: rule.severity.clone(),
                selector: selector.clone(),
                parameters,
            }),
            Err(groups) => deferred.push(DeferredRule {
                id,
                capability: definition.capability.clone(),
                reason: format!(
                    "capability `{}` evaluates one population; applicability names {groups} target groups",
                    definition.capability,
                ),
            }),
        }
    }
    Ok(ExecutionPlan {
        rules,
        deferred,
        concepts: Arc::new(concepts),
    })
}

/// Every rule definition the ruleset's declared packages provide, by ID.
fn definition_catalog<'a>(
    ruleset: &RuleSetPackage,
    packages: &BTreeMap<&str, &'a DefinitionPackage>,
) -> Result<BTreeMap<&'a str, &'a RuleDefinition>, EngineError> {
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
    Ok(catalog)
}

/// Binds a rule's parameters to its definition and to the trusted capability.
///
/// Applies declared defaults, then checks every binding against the
/// capability's descriptor and the definition's allowed values.
fn bind_parameters(
    registry: &CapabilityRegistry,
    rule: &RuleInstance,
    definition: &RuleDefinition,
) -> Result<BTreeMap<String, ParameterValue>, EngineError> {
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
        let descriptor = known
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
    Ok(parameters)
}

/// The one selector a capability evaluates, or the group count when there is none.
///
/// Every selector's concepts are validated either way, so an unknown concept
/// is a compile error even in a rule that will be deferred. One group names
/// exactly one population, the same one a flat selector would. With several,
/// a capability that takes one selector would have to pick a group or their
/// union, which evaluates a population the author did not name.
fn applicability_selector<'r>(
    concepts: &ConceptCatalog,
    rule: &'r RuleInstance,
) -> Result<Result<&'r Selector, usize>, EngineError> {
    match &rule.applicability {
        RuleApplicability::Selector(selector) => {
            validate_selector_concepts(concepts, &rule.id, selector)?;
            Ok(Ok(selector))
        }
        RuleApplicability::Groups(groups) => {
            for group in groups.groups.values() {
                validate_selector_concepts(concepts, &rule.id, &group.selector)?;
            }
            Ok(
                match groups.groups.values().collect::<Vec<_>>().as_slice() {
                    [only] => Ok(&only.selector),
                    _ => Err(groups.groups.len()),
                },
            )
        }
    }
}

/// Collects every concept the ruleset's declared definition packages provide.
fn concept_catalog(
    ruleset: &RuleSetPackage,
    packages: &BTreeMap<&str, &DefinitionPackage>,
) -> Result<ConceptCatalog, EngineError> {
    let mut catalog = ConceptCatalog::default();
    for package_id in &ruleset.definition_packages {
        let package = packages[package_id.as_str()];
        let entries = package
            .object_types
            .values()
            .map(|c| (ConceptKind::ObjectType, &c.id, &c.external_names))
            .chain(
                package
                    .properties
                    .values()
                    .map(|c| (ConceptKind::Property, &c.id, &c.external_names)),
            )
            .chain(
                package
                    .property_sets
                    .values()
                    .map(|c| (ConceptKind::PropertySet, &c.id, &c.external_names)),
            );
        for (kind, id, names) in entries {
            if catalog.insert(kind, id, names).is_err() {
                return Err(EngineError::DuplicateConcept(id.clone()));
            }
        }
    }
    Ok(catalog)
}

fn require_concept(
    concepts: &ConceptCatalog,
    rule: &str,
    kind: ConceptKind,
    concept: &str,
) -> Result<(), EngineError> {
    if concepts.contains(kind, concept) {
        Ok(())
    } else {
        Err(EngineError::UnknownConcept {
            rule: rule.into(),
            kind: kind.to_string(),
            concept: concept.into(),
        })
    }
}

/// A property-set qualifier is a declared concept, or a reserved attribute set.
///
/// The attribute sets are engine vocabulary, not package concepts: they bind
/// to the same meaning in every source, so a package cannot redeclare them.
fn require_set_concept(
    concepts: &ConceptCatalog,
    rule: &str,
    set: &str,
) -> Result<(), EngineError> {
    if axioval_ir::is_reserved_set(set) {
        return Ok(());
    }
    require_concept(concepts, rule, ConceptKind::PropertySet, set)
}

fn validate_selector_concepts(
    concepts: &ConceptCatalog,
    rule: &str,
    selector: &Selector,
) -> Result<(), EngineError> {
    match selector {
        Selector::All | Selector::Classification { .. } => Ok(()),
        Selector::EntityType { object_type, .. } => {
            require_concept(concepts, rule, ConceptKind::ObjectType, object_type)
        }
        Selector::Property {
            property_set,
            property,
            value,
            ..
        } => {
            require_concept(concepts, rule, ConceptKind::Property, property)?;
            if let Some(set) = property_set {
                require_set_concept(concepts, rule, set)?;
            }
            value
                .iter()
                .try_for_each(|value| validate_parameter_concepts(concepts, rule, value))
        }
        Selector::AllOf { operands } | Selector::AnyOf { operands } => operands
            .iter()
            .try_for_each(|operand| validate_selector_concepts(concepts, rule, operand)),
        Selector::Not { operand } => validate_selector_concepts(concepts, rule, operand),
    }
}

fn validate_parameter_concepts(
    concepts: &ConceptCatalog,
    rule: &str,
    value: &ParameterValue,
) -> Result<(), EngineError> {
    match value {
        ParameterValue::ObjectTypeReference { object_type, .. } => {
            require_concept(concepts, rule, ConceptKind::ObjectType, object_type)
        }
        ParameterValue::PropertyReference {
            property,
            property_set,
        } => {
            require_concept(concepts, rule, ConceptKind::Property, property)?;
            property_set
                .iter()
                .try_for_each(|set| require_set_concept(concepts, rule, set))
        }
        ParameterValue::Selector { value } => validate_selector_concepts(concepts, rule, value),
        _ => Ok(()),
    }
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
