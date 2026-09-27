//! Strict binding from portable schema packages to trusted executable plans.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use axioval_ir::contract::{
    ColumnKind, ParameterKind, ParameterValue, RuleApplicability, RuleDefinition, RuleFolder,
    RuleInstance, Selector, TableColumnDefinition, TableRow,
};
use axioval_ir::{DefinitionPackage, RuleId, RuleSetPackage};

use crate::concepts::{ConceptCatalog, ConceptKind};
use crate::refinement::{RuleRefinement, validate_bands};
use crate::{
    CapabilityRegistry, CompiledRule, DeferredRule, EngineError, ExecutionPlan,
    ParameterDescriptor, ParameterType, TableColumn,
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
    let mut refinements = BTreeMap::new();
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
        let refinement = refinement(registry, &concepts, rule, &definition.capability)?;
        if !refinement.is_empty() {
            refinements.insert(id.clone(), refinement);
        }
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
        refinements,
    })
}

/// What `rule` asks of its outcomes, checked against the capability.
fn refinement(
    registry: &CapabilityRegistry,
    concepts: &ConceptCatalog,
    rule: &RuleInstance,
    capability: &str,
) -> Result<RuleRefinement, EngineError> {
    let invalid = |detail: String| EngineError::InvalidRefinement {
        rule: rule.id.clone(),
        detail,
    };
    if !rule.severity_bands.is_empty() {
        validate_bands(&rule.severity_bands).map_err(invalid)?;
        let grades = registry
            .get(capability)
            .is_some_and(|capability| capability.grades_deviation());
        if !grades {
            return Err(invalid(format!(
                "capability `{capability}` reports no deviation to grade by `severityBands`"
            )));
        }
    }
    for entry in &rule.severity_overrides {
        validate_selector_concepts(concepts, &rule.id, &entry.selector)?;
    }
    for level in &rule.categories {
        require_concept(concepts, &rule.id, ConceptKind::Property, &level.property)?;
        if let Some(set) = &level.property_set {
            require_set_concept(concepts, &rule.id, set)?;
        }
    }
    let refinement = RuleRefinement {
        severity_bands: rule.severity_bands.clone(),
        severity_overrides: rule.severity_overrides.clone(),
        categories: rule.categories.clone(),
    };
    if refinement.needs_refiner() && registry.refiner().is_none() {
        return Err(invalid(
            "the rule refines its outcomes by reading the model, and the host registered no \
             outcome refiner"
                .into(),
        ));
    }
    Ok(refinement)
}

/// Separates a ruleset's package ID from a rule ID in a qualified rule ID.
pub const QUALIFIED_RULE_SEPARATOR: char = '/';

/// Compiles several rulesets into one plan, each rule ID qualified by its
/// ruleset's package ID.
///
/// Every ruleset is compiled on its own, against its own declared definition
/// packages, exactly as [`compile`] compiles it. Its rule IDs then become
/// `package-id/rule-id` ([`QUALIFIED_RULE_SEPARATOR`]), so two rulesets may
/// both define `r1` and report both findings under distinct IDs, and the
/// plan's ID order groups rules by package. One ruleset compiles as
/// [`compile`] does, with its IDs unqualified. The plan's concepts are those
/// of every definition package any ruleset declares.
///
/// # Errors
///
/// Returns every error [`compile`] returns for any ruleset, and an error
/// when no ruleset is given, two rulesets share a package ID, or two
/// declared definition packages declare one concept.
pub fn compile_rulesets(
    registry: &CapabilityRegistry,
    definitions: &[DefinitionPackage],
    rulesets: &[RuleSetPackage],
) -> Result<ExecutionPlan, EngineError> {
    let [first, rest @ ..] = rulesets else {
        return Err(EngineError::NoRuleSet);
    };
    if rest.is_empty() {
        return compile(registry, definitions, first);
    }
    let mut packages_seen = BTreeSet::new();
    let mut declared: Vec<&String> = Vec::new();
    let mut rules = Vec::new();
    let mut deferred = Vec::new();
    let mut refinements = BTreeMap::new();
    for ruleset in rulesets {
        let package = &ruleset.package.id;
        if !packages_seen.insert(package.as_str()) {
            return Err(EngineError::DuplicateRuleSet(package.clone()));
        }
        for id in &ruleset.definition_packages {
            if !declared.contains(&id) {
                declared.push(id);
            }
        }
        let plan = compile(registry, definitions, ruleset)?;
        let qualify = |id: &RuleId| {
            let qualified = format!("{package}{QUALIFIED_RULE_SEPARATOR}{id}");
            RuleId::new(qualified.clone()).map_err(|_| EngineError::InvalidRuleId(qualified))
        };
        for mut rule in plan.rules {
            rule.id = qualify(&rule.id)?;
            rules.push(rule);
        }
        for (id, refinement) in plan.refinements {
            refinements.insert(qualify(&id)?, refinement);
        }
        for mut rule in plan.deferred {
            rule.id = qualify(&rule.id)?;
            deferred.push(rule);
        }
    }
    rules.sort_by(|left, right| left.id.cmp(&right.id));
    deferred.sort_by(|left, right| left.id.cmp(&right.id));
    let packages = collect_definition_packages(definitions)?;
    let concepts = concepts_of(declared.into_iter(), &packages)?;
    Ok(ExecutionPlan {
        rules,
        deferred,
        concepts: Arc::new(concepts),
        refinements,
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
        if let (ParameterType::Table(columns), ParameterValue::Table { value: rows }) =
            (descriptor.parameter_type, value)
        {
            for (row, cells) in rows.iter().enumerate() {
                validate_row(columns, cells).map_err(|detail| EngineError::InvalidTableRow {
                    capability: definition.capability.clone(),
                    parameter: name.clone(),
                    row,
                    detail,
                })?;
            }
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
    concepts_of(ruleset.definition_packages.iter(), packages)
}

/// Every concept the packages `package_ids` names declare, each once.
fn concepts_of<'a>(
    package_ids: impl Iterator<Item = &'a String>,
    packages: &BTreeMap<&str, &DefinitionPackage>,
) -> Result<ConceptCatalog, EngineError> {
    let mut catalog = ConceptCatalog::default();
    for package_id in package_ids {
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
        Selector::All | Selector::Classification { .. } | Selector::Discipline { .. } => Ok(()),
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
        // Name patterns match source names and bind to no concept.
        Selector::PropertyPattern { value, .. } | Selector::Source { value, .. } => value
            .iter()
            .try_for_each(|value| validate_parameter_concepts(concepts, rule, value)),
        Selector::AllOf { operands } | Selector::AnyOf { operands } => operands
            .iter()
            .try_for_each(|operand| validate_selector_concepts(concepts, rule, operand)),
        Selector::Not { operand } => validate_selector_concepts(concepts, rule, operand),
        Selector::Related { selector, .. } => validate_selector_concepts(concepts, rule, selector),
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
        ParameterValue::Table { value: rows } => rows
            .iter()
            .flat_map(TableRow::values)
            .try_for_each(|cell| validate_parameter_concepts(concepts, rule, cell)),
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
            || !same_type(descriptor.parameter_type, &parameter.kind)
        {
            return contract_error(definition_id, capability_id, "parameter signature differs");
        }
        match descriptor.parameter_type {
            ParameterType::Table(columns) => {
                if !same_columns(columns, &parameter.columns) {
                    return contract_error(
                        definition_id,
                        capability_id,
                        &format!("table parameter `{}` columns differ", descriptor.name),
                    );
                }
                if !parameter.allowed_values.is_empty() {
                    return contract_error(
                        definition_id,
                        capability_id,
                        &format!(
                            "table parameter `{}` must not declare allowedValues",
                            descriptor.name
                        ),
                    );
                }
            }
            _ if !parameter.columns.is_empty() => {
                return contract_error(
                    definition_id,
                    capability_id,
                    &format!(
                        "only a table parameter declares columns, not `{}`",
                        descriptor.name
                    ),
                );
            }
            _ => {}
        }
    }
    Ok(())
}

/// Whether a definition's columns are the descriptor's, in any order.
///
/// Column names and descriptions are presentation; IDs, kinds and whether a
/// cell is required are the contract. Duplicate IDs never match.
fn same_columns(trusted: &[TableColumn], declared: &[TableColumnDefinition]) -> bool {
    let mut trusted: Vec<_> = trusted
        .iter()
        .map(|column| (column.id, column.kind, column.required))
        .collect();
    let mut declared: Vec<_> = declared
        .iter()
        .map(|column| (column.id.as_str(), column.kind, column.required))
        .collect();
    trusted.sort_unstable();
    declared.sort_unstable();
    let distinct = declared.windows(2).all(|pair| pair[0].0 != pair[1].0);
    distinct && trusted == declared
}

/// Checks one table row against the trusted columns.
fn validate_row(columns: &[TableColumn], row: &TableRow) -> Result<(), String> {
    for (id, cell) in row {
        let column = columns
            .iter()
            .find(|column| column.id == id)
            .ok_or_else(|| format!("unknown column `{id}`"))?;
        if !cell_fits(column.kind, cell) {
            return Err(format!(
                "column `{id}` takes a {} cell",
                column.kind.as_str()
            ));
        }
    }
    match columns
        .iter()
        .find(|column| column.required && !row.contains_key(column.id))
    {
        Some(column) => Err(format!("required column `{}` is empty", column.id)),
        None => Ok(()),
    }
}

fn cell_fits(kind: ColumnKind, cell: &ParameterValue) -> bool {
    match (kind, cell) {
        (ColumnKind::String, ParameterValue::String { .. })
        | (ColumnKind::Integer, ParameterValue::Integer { .. })
        | (ColumnKind::Boolean, ParameterValue::Boolean { .. })
        | (ColumnKind::Selector, ParameterValue::Selector { .. })
        | (ColumnKind::Reference, ParameterValue::Reference { .. })
        | (ColumnKind::Date, ParameterValue::Date { .. })
        | (ColumnKind::DateTime, ParameterValue::DateTime { .. }) => true,
        (ColumnKind::TextPattern, ParameterValue::String { value }) => well_formed_pattern(value),
        (ColumnKind::Number, ParameterValue::Number { value }) => value.is_finite(),
        (ColumnKind::Quantity, ParameterValue::Quantity { value, unit }) => {
            value.is_finite() && !unit.is_empty()
        }
        _ => false,
    }
}

/// A wildcard pattern whose every backslash escapes a following character.
fn well_formed_pattern(pattern: &str) -> bool {
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.next().is_none() {
            return false;
        }
    }
    true
}

fn same_type(parameter_type: ParameterType, kind: &ParameterKind) -> bool {
    match (parameter_type, kind) {
        (ParameterType::Table(_), ParameterKind::Table) => true,
        (ParameterType::Table(_), _) | (_, ParameterKind::Table) => false,
        (parameter_type, kind) => parameter_type == from_kind(kind),
    }
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
        ParameterKind::Date => ParameterType::Date,
        ParameterKind::DateTime => ParameterType::DateTime,
        ParameterKind::Reference => ParameterType::Reference,
        ParameterKind::ObjectTypeReference => ParameterType::ObjectTypeReference,
        ParameterKind::PropertyReference => ParameterType::PropertyReference,
        ParameterKind::Selector => ParameterType::Selector,
        ParameterKind::StringList => ParameterType::StringList,
        ParameterKind::ReferenceList => ParameterType::ReferenceList,
        ParameterKind::Table => ParameterType::Table(&[]),
    }
}
