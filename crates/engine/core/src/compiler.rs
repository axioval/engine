//! Strict binding from portable schema packages to trusted executable plans.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use axioval_ir::contract::{
    ClassTree, ClassificationDefinition, ClassificationProperty, ColumnKind, GateCondition,
    GroupingDefinition, GroupingKey, ParameterKind, ParameterValue, RelationDefinition,
    RelationKey, RuleApplicability, RuleDefinition, RuleFolder, RuleGate, RuleInstance, Selector,
    TableColumnDefinition, TableFileColumn, TableRow,
};
use axioval_ir::{DefinitionPackage, RuleId, RuleSetPackage};

use crate::concepts::{ConceptCatalog, ConceptKind};
use crate::refinement::{RuleRefinement, validate_bands};
use crate::rule_outcomes;
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
    let concepts = declared_concepts(ruleset, &packages)?;
    let (classifications, groupings, relations) = derivations(registry, &concepts, ruleset)?;
    let properties = vocabulary_properties(ruleset, &packages);
    let (vocabulary, values) =
        crate::expression_binding::check_values(&concepts, &properties, ruleset)?;
    let mut authored = Vec::new();
    flatten(&ruleset.root, &[], &mut authored);
    authored.sort_by(|(left, _), (right, _)| left.id.cmp(&right.id));
    let known: BTreeSet<&str> = authored.iter().map(|(rule, _)| rule.id.as_str()).collect();
    let disabled: BTreeSet<&str> = authored
        .iter()
        .filter(|(rule, _)| !rule.enabled)
        .map(|(rule, _)| rule.id.as_str())
        .collect();
    let mut ids = BTreeSet::new();
    let mut rules = Vec::new();
    let mut deferred = Vec::new();
    let mut refinements = BTreeMap::new();
    let mut gates = BTreeMap::new();
    let mut dependencies: BTreeMap<RuleId, BTreeSet<RuleId>> = BTreeMap::new();
    let mut recorded = BTreeSet::new();
    let mut auxiliary = BTreeSet::new();
    for (rule, folder_gates) in authored.into_iter().filter(|(rule, _)| rule.enabled) {
        if !ids.insert(rule.id.as_str()) {
            return Err(EngineError::DuplicateRule(rule.id.clone()));
        }
        let definition = catalog
            .get(rule.definition_id.as_str())
            .ok_or_else(|| EngineError::UnknownDefinition(rule.definition_id.clone()))?;
        let parameters = bind_parameters(registry, rule, definition)?;
        validate_bound(&vocabulary, &rule.id, &parameters, definition)?;
        let id = RuleId::new(rule.id.clone())
            .map_err(|_| EngineError::InvalidRuleId(rule.id.clone()))?;
        let refinement = refinement(registry, &concepts, rule, &definition.capability)?;
        let dependency = rule_dependencies(rule, &folder_gates, &parameters, &refinement, &known)?;
        recorded.extend(dependency.per_object.iter().cloned());
        if rule.auxiliary {
            auxiliary.insert(id.clone());
        }
        if !dependency.whole.is_empty() {
            gates.insert(id.clone(), dependency.whole);
        }
        dependencies.insert(id.clone(), dependency.all);
        if !refinement.is_empty() {
            refinements.insert(id.clone(), refinement);
        }
        match applicability_selector(&concepts, rule)? {
            Ok(selector) => rules.push(CompiledRule {
                id,
                capability: definition.capability.clone(),
                severity: rule.severity.clone(),
                selector: gated(selector, dependency.narrowing),
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
    // An auxiliary rule reports only through the rules that read it, so
    // one nothing reads would hide its outcome entirely.
    if let Some(unread) = auxiliary
        .iter()
        .find(|id| !dependencies.values().any(|parents| parents.contains(*id)))
    {
        return Err(EngineError::InvalidDependency {
            rule: unread.to_string(),
            detail: "the rule is auxiliary, but no enabled rule reads its outcome, so it \
                     would never be reported"
                .into(),
        });
    }
    let rules = defer_dependents(rules, &mut deferred, &dependencies, &disabled);
    deferred.sort_by(|left, right| left.id.cmp(&right.id));
    let rules = ordered(registry, rules, &dependencies, &recorded)?;
    Ok(ExecutionPlan {
        rules,
        deferred,
        concepts: Arc::new(concepts),
        refinements,
        gates,
        recorded,
        auxiliary,
        classifications,
        groupings,
        relations,
        supplied: BTreeMap::new(),
        values: Arc::new(values),
    })
}

/// The ruleset's classifications, groupings and relations, each checked.
fn derivations(
    registry: &CapabilityRegistry,
    concepts: &ConceptCatalog,
    ruleset: &RuleSetPackage,
) -> Result<Derived, EngineError> {
    Ok((
        classifications(registry, concepts, ruleset)?,
        groupings(registry, concepts, ruleset)?,
        relations(registry, concepts, ruleset)?,
    ))
}

/// The ruleset's relations, checked: each declared under its id, an id
/// usable in an identity, both selectors naming declared concepts and
/// reading neither a rule's outcome nor a declared relation (relations are
/// derived before any rule runs), and its key either two declared
/// properties or a table of text pairs, which is bound inline.
fn relations(
    registry: &CapabilityRegistry,
    concepts: &ConceptCatalog,
    ruleset: &RuleSetPackage,
) -> Result<Vec<RelationDefinition>, EngineError> {
    let mut checked = Vec::new();
    for (key, definition) in &ruleset.relations {
        let invalid = |detail: String| EngineError::InvalidRelation {
            relation: key.clone(),
            detail,
        };
        if *key != definition.id {
            return Err(invalid(format!(
                "is declared under the key `{key}`, not its id"
            )));
        }
        if definition.id.trim().is_empty()
            || definition
                .id
                .chars()
                .any(|c| matches!(c, ':' | ';' | '|' | '/') || c.is_whitespace())
        {
            return Err(invalid(
                "its id must not be blank or hold `:`, `;`, `|`, `/` or whitespace".into(),
            ));
        }
        if registry.refiner().is_none() {
            return Err(invalid(
                "the host registered no outcome refiner to select its objects".into(),
            ));
        }
        for (end, selector) in [("from", &definition.from), ("to", &definition.to)] {
            validate_selector_concepts(concepts, &format!("{}#{end}", definition.id), selector)?;
            let mut rules = BTreeSet::new();
            rule_outcomes::selector_references(selector, &mut rules);
            if !rules.is_empty() || reads_relations(selector) {
                return Err(invalid(format!(
                    "its `{end}` reads a rule's outcome or a declared relation; relations are \
                     derived before any rule runs"
                )));
            }
        }
        let mut definition = definition.clone();
        match &mut definition.by {
            RelationKey::Property { from, to } => {
                for (end, property) in [("from", &*from), ("to", &*to)] {
                    require_property(
                        concepts,
                        &format!("{}#{end}", definition.id),
                        property.property_set.as_deref(),
                        &property.property,
                    )?;
                }
            }
            RelationKey::Pairs { pairs, scheme } => {
                if scheme
                    .as_ref()
                    .is_some_and(|scheme| scheme.trim().is_empty())
                {
                    return Err(invalid("its external id scheme is blank".into()));
                }
                *pairs = relation_pairs(pairs).map_err(invalid)?;
            }
            RelationKey::Supplied { columns, scheme } => {
                if scheme
                    .as_ref()
                    .is_some_and(|scheme| scheme.trim().is_empty())
                {
                    return Err(invalid("its external id scheme is blank".into()));
                }
                // Stated in full, so rulesets declaring it alike merge.
                *columns = Some(supplied_columns(columns.as_deref()).map_err(invalid)?);
            }
        }
        checked.push(definition);
    }
    Ok(checked)
}

/// The columns a supplied relation's pairs are read by: `columns` when
/// declared (exactly the text columns `from` and `to`, each once, headers
/// distinct and not empty), else the headers `from` and `to`.
pub(crate) fn supplied_columns(
    columns: Option<&[TableFileColumn]>,
) -> Result<Vec<TableFileColumn>, String> {
    let Some(columns) = columns else {
        return Ok(["from", "to"]
            .map(|id| TableFileColumn {
                id: id.into(),
                header: None,
                kind: ColumnKind::String,
                unit: None,
            })
            .into());
    };
    for column in columns {
        if !matches!(column.id.as_str(), "from" | "to") {
            return Err(format!(
                "its supplied column `{}` is neither `from` nor `to`",
                column.id
            ));
        }
        if column.kind != ColumnKind::String {
            return Err(format!(
                "its supplied column `{}` is declared {}, not string",
                column.id,
                column.kind.as_str()
            ));
        }
    }
    crate::table_files::check_columns(columns)
        .map_err(|detail| format!("its supplied columns: {detail}"))?;
    if let Some(missing) = ["from", "to"]
        .into_iter()
        .find(|id| !columns.iter().any(|column| column.id == *id))
    {
        return Err(format!("its supplied columns declare no `{missing}`"));
    }
    Ok(columns.to_vec())
}

/// Every row of a relation's pairs holds exactly a non-blank text `from`
/// and `to` cell.
pub(crate) fn check_pair_rows(rows: &[TableRow]) -> Result<(), String> {
    for (index, row) in rows.iter().enumerate() {
        validate_row(&trusted_columns(RELATION_PAIR_COLUMNS), row)
            .map_err(|detail| format!("pairs row {}: {detail}", index + 1))?;
        if row
            .values()
            .any(|cell| matches!(cell, ParameterValue::String { value } if value.trim().is_empty()))
        {
            return Err(format!("pairs row {} names a blank object", index + 1));
        }
    }
    Ok(())
}

/// The columns of a relation's `pairs` table.
const RELATION_PAIR_COLUMNS: &[TableColumn] = &[
    TableColumn::required("from", ColumnKind::String),
    TableColumn::required("to", ColumnKind::String),
];

/// A relation's pairs as an inline table of non-blank text `from` and `to`
/// cells, refusing any other value, an unloaded table file and a table
/// file declaring other columns.
fn relation_pairs(pairs: &ParameterValue) -> Result<ParameterValue, String> {
    let pairs = match pairs {
        ParameterValue::TableFile(_) => bind_table_file(
            "relation",
            "pairs",
            &ParameterDescriptor::required("pairs", ParameterType::Table(RELATION_PAIR_COLUMNS)),
            pairs,
        )
        .map_err(|error| match error {
            EngineError::InvalidTableFile { path, detail, .. } => {
                format!("its pairs file `{path}`: {detail}")
            }
            other => other.to_string(),
        })?,
        ParameterValue::Table { .. } => pairs.clone(),
        _ => return Err("its pairs must be a table or a table file".into()),
    };
    let ParameterValue::Table { value: rows } = &pairs else {
        return Err("its pairs must be a table or a table file".into());
    };
    check_pair_rows(rows)?;
    Ok(pairs)
}

/// Whether `selector` walks a declared relation.
fn reads_relations(selector: &Selector) -> bool {
    match selector {
        Selector::Related { path, selector, .. } => {
            path.iter()
                .any(|step| step.contains(crate::RELATION_RELATIONSHIP_PREFIX))
                || reads_relations(selector)
        }
        Selector::AllOf { operands } | Selector::AnyOf { operands } => {
            operands.iter().any(reads_relations)
        }
        Selector::Not { operand } => reads_relations(operand),
        _ => false,
    }
}

/// The ruleset's groupings, checked: each declared under its id, an id
/// usable in an identity (no `:`, `;`, `|` or `/`, not blank), members and
/// key naming declared concepts, and neither reading a rule's outcome nor a
/// derived group, since groups are derived before any rule runs and from
/// the model alone.
fn groupings(
    registry: &CapabilityRegistry,
    concepts: &ConceptCatalog,
    ruleset: &RuleSetPackage,
) -> Result<Vec<GroupingDefinition>, EngineError> {
    let mut checked = Vec::new();
    for (key, definition) in &ruleset.groupings {
        let invalid = |detail: String| EngineError::InvalidGrouping {
            grouping: key.clone(),
            detail,
        };
        if *key != definition.id {
            return Err(invalid(format!(
                "is declared under the key `{key}`, not its id"
            )));
        }
        if definition.id.trim().is_empty()
            || definition
                .id
                .chars()
                .any(|c| matches!(c, ':' | ';' | '|' | '/') || c.is_whitespace())
        {
            return Err(invalid(
                "its id must not be blank or hold `:`, `;`, `|`, `/` or whitespace".into(),
            ));
        }
        if registry.refiner().is_none() {
            return Err(invalid(
                "the host registered no outcome refiner to select its members".into(),
            ));
        }
        let context = format!("{}#members", definition.id);
        validate_selector_concepts(concepts, &context, &definition.members)?;
        let mut rules = BTreeSet::new();
        rule_outcomes::selector_references(&definition.members, &mut rules);
        if !rules.is_empty() {
            return Err(invalid(
                "its members read a rule's outcome; groups are derived before any rule runs".into(),
            ));
        }
        if reads_groups(&definition.members) {
            return Err(invalid(
                "its members read derived groups; groups are derived from the model alone".into(),
            ));
        }
        match &definition.by {
            GroupingKey::Property {
                property_set,
                property,
            } => {
                if property_set.as_deref() == Some(axioval_ir::GROUP_SET) {
                    return Err(invalid("it groups by a derived group's own facts".into()));
                }
                require_property(concepts, &context, property_set.as_deref(), property)?;
            }
            GroupingKey::Classification { system } => {
                if system.trim().is_empty() {
                    return Err(invalid("its classification system is blank".into()));
                }
            }
            GroupingKey::Compartment {
                separators,
                boundary,
                tolerance,
                overlap,
            } => {
                if tolerance.is_some_and(|value| !value.is_finite() || value < 0.0)
                    || overlap.is_some_and(|value| !value.is_finite() || value <= 0.0)
                {
                    return Err(invalid(
                        "its tolerance must be finite and at least zero, its overlap finite and \
                         positive"
                            .into(),
                    ));
                }
                for (part, selector) in [("separators", separators), ("boundary", boundary)] {
                    validate_selector_concepts(
                        concepts,
                        &format!("{}#{part}", definition.id),
                        selector,
                    )?;
                    let mut rules = BTreeSet::new();
                    rule_outcomes::selector_references(selector, &mut rules);
                    if !rules.is_empty() || reads_groups(selector) {
                        return Err(invalid(format!(
                            "its {part} read a rule's outcome or a derived group"
                        )));
                    }
                }
            }
        }
        checked.push(definition.clone());
    }
    Ok(checked)
}

/// Whether `selector` reads derived groups: a `derivedGroup` selector or a
/// property of the reserved group set.
fn reads_groups(selector: &Selector) -> bool {
    match selector {
        Selector::DerivedGroup { .. } => true,
        Selector::Property {
            property_set: Some(set),
            ..
        } => set == axioval_ir::GROUP_SET,
        Selector::AllOf { operands } | Selector::AnyOf { operands } => {
            operands.iter().any(reads_groups)
        }
        Selector::Not { operand } => reads_groups(operand),
        Selector::Related { selector, .. } => reads_groups(selector),
        _ => false,
    }
}

/// The class tree of every classification, by id, refusing a
/// classification declared under another key than its id, one without an
/// id or rows, and a malformed tree ([`ClassTree::of`]).
fn class_trees<'a>(
    classifications: impl Iterator<Item = (&'a String, &'a ClassificationDefinition)>,
) -> Result<Vec<(String, ClassTree)>, EngineError> {
    classifications
        .map(|(key, definition)| {
            let invalid = |detail: String| EngineError::InvalidClassification {
                classification: key.clone(),
                detail,
            };
            if *key != definition.id {
                return Err(invalid(format!(
                    "is declared under the key `{key}`, not its id"
                )));
            }
            if definition.id.trim().is_empty() {
                return Err(invalid("its id is blank".into()));
            }
            if definition.rows.is_empty() {
                return Err(invalid("it has no rows".into()));
            }
            let tree = ClassTree::of(definition).map_err(invalid)?;
            Ok((key.clone(), tree))
        })
        .collect()
}

/// The ruleset's classifications, checked and ordered so each follows the
/// classifications its rows read.
fn classifications(
    registry: &CapabilityRegistry,
    concepts: &ConceptCatalog,
    ruleset: &RuleSetPackage,
) -> Result<Vec<ClassificationDefinition>, EngineError> {
    let mut read_by: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (key, definition) in &ruleset.classifications {
        let invalid = |detail: String| EngineError::InvalidClassification {
            classification: key.clone(),
            detail,
        };
        if registry.refiner().is_none() {
            return Err(invalid(
                "the host registered no outcome refiner to evaluate its rows".into(),
            ));
        }
        let mut read = BTreeSet::new();
        for (index, row) in definition.rows.iter().enumerate() {
            if row.class.trim().is_empty() {
                return Err(invalid(format!("row {index} assigns a blank class")));
            }
            let context = format!("{}#{index}", definition.id);
            validate_selector_concepts(concepts, &context, &row.selector)?;
            let mut rules = BTreeSet::new();
            rule_outcomes::selector_references(&row.selector, &mut rules);
            if !rules.is_empty() {
                return Err(invalid(format!(
                    "row {index} reads a rule's outcome; classes are derived before any rule runs"
                )));
            }
            if reads_groups(&row.selector) {
                return Err(invalid(format!(
                    "row {index} reads derived groups, which are derived after the classes"
                )));
            }
            classifications_read(&row.selector, &mut read);
        }
        read_by.insert(definition.id.as_str(), read);
    }
    let mut ordered: Vec<ClassificationDefinition> = Vec::new();
    let mut pending: BTreeSet<&str> = read_by.keys().copied().collect();
    while !pending.is_empty() {
        let next = pending
            .iter()
            .copied()
            .find(|id| read_by[id].iter().all(|needed| !pending.contains(needed)));
        let Some(next) = next else {
            let first = pending.iter().next().copied().unwrap_or_default();
            return Err(EngineError::InvalidClassification {
                classification: first.to_owned(),
                detail: format!(
                    "the classifications {} read one another in a cycle",
                    pending
                        .iter()
                        .map(|id| format!("`{id}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        };
        pending.remove(next);
        ordered.push(ruleset.classifications[next].clone());
    }
    Ok(ordered)
}

/// Every classification a property or derived-class selector in
/// `selector` reads.
fn classifications_read<'a>(selector: &'a Selector, out: &mut BTreeSet<&'a str>) {
    match selector {
        Selector::Property {
            property_set: Some(set),
            property,
            ..
        } if set == axioval_ir::CLASSIFICATION_SET => {
            let read = ClassificationProperty::parse(property)
                .map_or(property.as_str(), |read| read.classification);
            out.insert(read);
        }
        Selector::DerivedClass { classification, .. } => {
            out.insert(classification);
        }
        Selector::AllOf { operands } | Selector::AnyOf { operands } => {
            for operand in operands {
                classifications_read(operand, out);
            }
        }
        Selector::Not { operand } => classifications_read(operand, out),
        Selector::Related { selector, .. } => classifications_read(selector, out),
        _ => {}
    }
}

/// `rules` in dependency order, refused when they form a cycle or read
/// another rule per object without a refiner to record its selection.
fn ordered(
    registry: &CapabilityRegistry,
    rules: Vec<CompiledRule>,
    dependencies: &BTreeMap<RuleId, BTreeSet<RuleId>>,
    recorded: &BTreeSet<RuleId>,
) -> Result<Vec<CompiledRule>, EngineError> {
    let rules = rule_outcomes::dependency_order(rules, dependencies).map_err(|cycle| {
        EngineError::InvalidDependency {
            rule: cycle[0].to_string(),
            detail: format!(
                "the rules {} depend on one another's outcomes in a cycle",
                cycle
                    .iter()
                    .map(|id| format!("`{id}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    })?;
    if let Some(reader) = rules.iter().find(|rule| {
        dependencies[&rule.id]
            .iter()
            .any(|parent| recorded.contains(parent))
    }) && registry.refiner().is_none()
    {
        return Err(EngineError::InvalidDependency {
            rule: reader.id.to_string(),
            detail: "the rule reads another rule's outcomes per object, and the host \
                     registered no outcome refiner to record that rule's selection"
                .into(),
        });
    }
    Ok(rules)
}

/// What one rule reads of other rules' outcomes.
struct RuleDependency {
    /// Every rule it reads, as a whole or per object.
    all: BTreeSet<RuleId>,
    /// Rules it reads per object, whose selection must be recorded.
    per_object: BTreeSet<RuleId>,
    /// Whole-rule gates, parent and condition.
    whole: Vec<(RuleId, GateCondition)>,
    /// Selectors its object gates narrow its applicability by.
    narrowing: Vec<Selector>,
}

/// The rules `rule` depends on through its folders' gates and its own, and
/// through `ruleOutcome` selectors in its applicability, parameters and
/// severity overrides. Every one must be a rule of the ruleset, and never
/// the rule itself.
fn rule_dependencies(
    rule: &RuleInstance,
    folder_gates: &[&RuleGate],
    parameters: &BTreeMap<String, ParameterValue>,
    refinement: &RuleRefinement,
    known: &BTreeSet<&str>,
) -> Result<RuleDependency, EngineError> {
    let invalid = |detail: String| EngineError::InvalidDependency {
        rule: rule.id.clone(),
        detail,
    };
    let mut per_object: BTreeSet<&str> = BTreeSet::new();
    match &rule.applicability {
        RuleApplicability::Selector(selector) => {
            rule_outcomes::selector_references(selector, &mut per_object);
        }
        RuleApplicability::Groups(groups) => {
            for group in groups.groups.values() {
                rule_outcomes::selector_references(&group.selector, &mut per_object);
            }
        }
    }
    for value in parameters.values() {
        rule_outcomes::value_references(value, &mut per_object);
    }
    for entry in &refinement.severity_overrides {
        rule_outcomes::selector_references(&entry.selector, &mut per_object);
    }
    let mut whole = Vec::new();
    let mut narrowing = Vec::new();
    for gate in folder_gates.iter().copied().chain(&rule.gate) {
        match rule_outcomes::gate_selector(&gate.rule, gate.condition) {
            Some(selector) => {
                per_object.insert(&gate.rule);
                narrowing.push(selector);
            }
            None => whole.push((gate.rule.as_str(), gate.condition)),
        }
    }
    let rule_id = |name: &str| {
        if name == rule.id {
            return Err(invalid(
                "the rule depends on its own outcome; a gate on a folder must name a rule \
                 outside it"
                    .into(),
            ));
        }
        if !known.contains(name) {
            return Err(invalid(format!(
                "the rule depends on rule `{name}`, which the ruleset does not define"
            )));
        }
        RuleId::new(name).map_err(|_| EngineError::InvalidRuleId(name.into()))
    };
    let per_object = per_object
        .into_iter()
        .map(rule_id)
        .collect::<Result<BTreeSet<_>, _>>()?;
    let whole = whole
        .into_iter()
        .map(|(name, condition)| Ok((rule_id(name)?, condition)))
        .collect::<Result<Vec<_>, EngineError>>()?;
    let mut all = per_object.clone();
    all.extend(whole.iter().map(|(parent, _)| parent.clone()));
    Ok(RuleDependency {
        all,
        per_object,
        whole,
        narrowing,
    })
}

/// `selector` narrowed by an object gate's selectors, gates first.
fn gated(selector: &Selector, narrowing: Vec<Selector>) -> Selector {
    if narrowing.is_empty() {
        return selector.clone();
    }
    let mut operands = narrowing;
    operands.push(selector.clone());
    Selector::AllOf { operands }
}

/// `rules` without those depending, directly or through other rules, on a
/// disabled or deferred rule; those are deferred, since their parent never
/// runs and their gate or selection could never be decided.
fn defer_dependents(
    mut rules: Vec<CompiledRule>,
    deferred: &mut Vec<DeferredRule>,
    dependencies: &BTreeMap<RuleId, BTreeSet<RuleId>>,
    disabled: &BTreeSet<&str>,
) -> Vec<CompiledRule> {
    let blocking = |parent: &RuleId, deferred: &[DeferredRule]| {
        disabled.contains(parent.to_string().as_str()) || deferred.iter().any(|d| d.id == *parent)
    };
    while let Some(index) = rules.iter().position(|rule| {
        dependencies[&rule.id]
            .iter()
            .any(|parent| blocking(parent, deferred))
    }) {
        let rule = rules.remove(index);
        let parents = dependencies[&rule.id]
            .iter()
            .filter(|parent| blocking(parent, deferred))
            .map(|parent| format!("`{parent}`"))
            .collect::<Vec<_>>()
            .join(", ");
        deferred.push(DeferredRule {
            id: rule.id,
            capability: rule.capability,
            reason: format!(
                "the rule depends on the outcome of rule {parents}, which is disabled or \
                 cannot run"
            ),
        });
    }
    rules
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
        require_property(
            concepts,
            &rule.id,
            level.property_set.as_deref(),
            &level.property,
        )?;
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
    let mut by_package: BTreeMap<&str, Vec<CompiledRule>> = BTreeMap::new();
    let mut deferred = Vec::new();
    let mut refinements = BTreeMap::new();
    let mut gates = BTreeMap::new();
    let mut recorded = BTreeSet::new();
    let mut auxiliary = BTreeSet::new();
    let mut derived: Derived = (Vec::new(), Vec::new(), Vec::new());
    let mut values: BTreeMap<String, axioval_ir::contract::Expression> = BTreeMap::new();
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
        // A ruleset's rules read only its own rules' outcomes, by the same
        // qualified ids.
        let rename = |name: &str| format!("{package}{QUALIFIED_RULE_SEPARATOR}{name}");
        for mut rule in plan.rules {
            rule.id = qualify(&rule.id)?;
            rule_outcomes::rename_selector(&mut rule.selector, &rename);
            for value in rule.parameters.values_mut() {
                rule_outcomes::rename_value(value, &rename);
            }
            by_package.entry(package).or_default().push(rule);
        }
        for (id, mut refinement) in plan.refinements {
            for entry in &mut refinement.severity_overrides {
                rule_outcomes::rename_selector(&mut entry.selector, &rename);
            }
            refinements.insert(qualify(&id)?, refinement);
        }
        for (id, parents) in plan.gates {
            let parents = parents
                .into_iter()
                .map(|(parent, condition)| Ok((qualify(&parent)?, condition)))
                .collect::<Result<Vec<_>, EngineError>>()?;
            gates.insert(qualify(&id)?, parents);
        }
        for id in plan.recorded {
            recorded.insert(qualify(&id)?);
        }
        for id in plan.auxiliary {
            auxiliary.insert(qualify(&id)?);
        }
        merge_derived(
            &mut derived,
            (plan.classifications, plan.groupings, plan.relations),
        )?;
        merge_values(&mut values, &plan.values)?;
        for mut rule in plan.deferred {
            rule.id = qualify(&rule.id)?;
            deferred.push(rule);
        }
    }
    // Each ruleset's rules keep their dependency order; rulesets follow one
    // another by package ID, as their qualified IDs sort.
    let rules: Vec<CompiledRule> = by_package.into_values().flatten().collect();
    deferred.sort_by(|left, right| left.id.cmp(&right.id));
    let (classifications, groupings, relations) = derived;
    let packages = collect_definition_packages(definitions)?;
    let mut concepts = concepts_of(declared.into_iter(), &packages)?;
    concepts.declare_classifications(class_trees(
        classifications
            .iter()
            .map(|definition| (&definition.id, definition)),
    )?);
    concepts.declare_groupings(groupings.iter().map(|definition| definition.id.clone()));
    concepts.declare_values(values.keys().cloned());
    Ok(ExecutionPlan {
        rules,
        deferred,
        concepts: Arc::new(concepts),
        refinements,
        gates,
        recorded,
        auxiliary,
        classifications,
        groupings,
        relations,
        supplied: BTreeMap::new(),
        values: Arc::new(values),
    })
}

/// What a run derives before any rule: classifications, groupings and
/// relations.
type Derived = (
    Vec<ClassificationDefinition>,
    Vec<GroupingDefinition>,
    Vec<RelationDefinition>,
);

/// Adds one ruleset's derived definitions to those of the rulesets before
/// it. One run derives one class per classification id, one set of groups
/// per grouping id and one relation per relation id, so rulesets share
/// each only when they declare it alike.
fn merge_derived(
    (classifications, groupings, relations): &mut Derived,
    (more_classifications, more_groupings, more_relations): Derived,
) -> Result<(), EngineError> {
    merge(classifications, more_classifications, |known| &known.id).map_err(|id| {
        EngineError::InvalidClassification {
            classification: id,
            detail: "two rulesets declare it with different rows or classes".into(),
        }
    })?;
    merge(groupings, more_groupings, |known| &known.id).map_err(|id| {
        EngineError::InvalidGrouping {
            grouping: id,
            detail: "two rulesets declare it differently".into(),
        }
    })?;
    merge(relations, more_relations, |known| &known.id).map_err(|id| EngineError::InvalidRelation {
        relation: id,
        detail: "two rulesets declare it differently".into(),
    })
}

/// Adds each of `more` to `known` unless an equal one is there; `Err` with
/// the id of one declared differently.
fn merge<T: PartialEq>(
    known: &mut Vec<T>,
    more: Vec<T>,
    id: impl Fn(&T) -> &String,
) -> Result<(), String> {
    for definition in more {
        match known.iter().find(|seen| id(seen) == id(&definition)) {
            Some(seen) if *seen == definition => {}
            Some(_) => return Err(id(&definition).clone()),
            None => known.push(definition),
        }
    }
    Ok(())
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
    let authored = capability.takes_authored_parameters();
    validate_signature(
        &rule.definition_id,
        &definition.capability,
        &descriptors,
        &definition.parameters,
        authored,
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
    for (name, value) in &mut parameters {
        if let (Some(descriptor), ParameterValue::TableFile(_)) =
            (known.get(name.as_str()), &*value)
        {
            *value = bind_table_file(&definition.capability, name, descriptor, value)?;
        }
    }
    for (name, value) in &parameters {
        let Some(descriptor) = known.get(name.as_str()) else {
            match definition.parameters.get(name) {
                Some(declared) if authored => {
                    bind_authored(&definition.capability, name, declared, value)?;
                    continue;
                }
                _ => {
                    return Err(EngineError::UnknownParameter {
                        capability: definition.capability.clone(),
                        parameter: name.clone(),
                    });
                }
            }
        };
        if !descriptor.parameter_type.accepts(value) {
            return Err(EngineError::InvalidParameterType {
                capability: definition.capability.clone(),
                parameter: name.clone(),
            });
        }
        if let (ParameterType::Table(columns), ParameterValue::Table { value: rows }) =
            (descriptor.parameter_type, value)
        {
            let columns = trusted_columns(columns);
            for (row, cells) in rows.iter().enumerate() {
                validate_row(&columns, cells).map_err(|detail| EngineError::InvalidTableRow {
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

/// Checks a parameter a definition declares beyond the capability's own
/// (`RuleCapability::takes_authored_parameters`) against its declared kind,
/// and a table's rows against its declared columns.
fn bind_authored(
    capability: &str,
    name: &str,
    declared: &axioval_ir::contract::ParameterDefinition,
    value: &ParameterValue,
) -> Result<(), EngineError> {
    let invalid = || EngineError::InvalidParameterType {
        capability: capability.into(),
        parameter: name.into(),
    };
    if declared.kind == ParameterKind::Table {
        let ParameterValue::Table { value: rows } = value else {
            return Err(invalid());
        };
        let columns: Vec<_> = declared
            .columns
            .iter()
            .map(|column| (column.id.as_str(), column.kind, column.required))
            .collect();
        for (row, cells) in rows.iter().enumerate() {
            validate_row(&columns, cells).map_err(|detail| EngineError::InvalidTableRow {
                capability: capability.into(),
                parameter: name.into(),
                row,
                detail,
            })?;
        }
        return Ok(());
    }
    if from_kind(&declared.kind).accepts(value) {
        Ok(())
    } else {
        Err(invalid())
    }
}

/// The rows of a loaded table file, bound as the same rows written inline.
///
/// Refused when the file was not loaded, the parameter is not a table, a
/// declared column is not one of the table's or has another kind, or a
/// required column of the table is not declared.
fn bind_table_file(
    capability: &str,
    parameter: &str,
    descriptor: &ParameterDescriptor,
    value: &ParameterValue,
) -> Result<ParameterValue, EngineError> {
    let ParameterValue::TableFile(file) = value else {
        return Ok(value.clone());
    };
    let axioval_ir::contract::TableFileReference {
        path,
        columns: declared,
        rows,
        ..
    } = &**file;
    let refuse = |detail: String| EngineError::InvalidTableFile {
        capability: capability.into(),
        parameter: parameter.into(),
        path: path.clone(),
        detail,
    };
    let ParameterType::Table(columns) = descriptor.parameter_type else {
        return Err(EngineError::InvalidParameterType {
            capability: capability.into(),
            parameter: parameter.into(),
        });
    };
    let Some(rows) = rows else {
        return Err(refuse(
            "the file was not loaded; load the package's table files before binding it".into(),
        ));
    };
    for column in declared {
        let trusted = columns
            .iter()
            .find(|trusted| trusted.id == column.id)
            .ok_or_else(|| refuse(format!("`{}` is not a column of the table", column.id)))?;
        if trusted.kind != column.kind {
            return Err(refuse(format!(
                "column `{}` is declared {} but the table's is {}",
                column.id,
                column.kind.as_str(),
                trusted.kind.as_str()
            )));
        }
    }
    if let Some(missing) = columns
        .iter()
        .find(|trusted| trusted.required && !declared.iter().any(|column| column.id == trusted.id))
    {
        return Err(refuse(format!(
            "the table's required column `{}` is not declared",
            missing.id
        )));
    }
    Ok(ParameterValue::Table {
        value: rows.clone(),
    })
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

/// A property reference: a declared property concept in a declared set or
/// a reserved one, or, in a derived set, a name the engine derives there.
pub(crate) fn require_property(
    concepts: &ConceptCatalog,
    rule: &str,
    set: Option<&str>,
    property: &str,
) -> Result<(), EngineError> {
    if set == Some(axioval_ir::MEASURED_SET) {
        return crate::measured::parse(property)
            .map(drop)
            .map_err(|detail| EngineError::InvalidMeasured {
                rule: rule.into(),
                property: property.into(),
                detail,
            });
    }
    if let Some(set) = set {
        match concepts.derives(set, property) {
            Some(true) => return Ok(()),
            Some(false) => {
                return Err(EngineError::UnknownConcept {
                    rule: rule.into(),
                    kind: set.into(),
                    concept: property.into(),
                });
            }
            None => require_set_concept(concepts, rule, set)?,
        }
    }
    require_concept(concepts, rule, ConceptKind::Property, property)
}

fn validate_selector_concepts(
    concepts: &ConceptCatalog,
    rule: &str,
    selector: &Selector,
) -> Result<(), EngineError> {
    match selector {
        // A rule reference is checked with the rule's dependencies.
        Selector::All
        | Selector::Classification { .. }
        | Selector::Discipline { .. }
        | Selector::RuleOutcome { .. } => Ok(()),
        // Its types are checked once the ruleset's values are known.
        Selector::Expression { expression } => {
            crate::expression_binding::expression_concepts(concepts, rule, expression)
        }
        Selector::EntityType { object_type, .. } => {
            require_concept(concepts, rule, ConceptKind::ObjectType, object_type)
        }
        Selector::DerivedClass {
            classification,
            class,
            ..
        } => match concepts.classification(classification) {
            None => Err(EngineError::UnknownConcept {
                rule: rule.into(),
                kind: axioval_ir::CLASSIFICATION_SET.into(),
                concept: classification.clone(),
            }),
            Some(tree) if !tree.contains(class) => Err(EngineError::UnknownConcept {
                rule: rule.into(),
                kind: format!("{} class", axioval_ir::CLASSIFICATION_SET),
                concept: format!("{classification}/{class}"),
            }),
            Some(_) => Ok(()),
        },
        Selector::DerivedGroup { grouping } => {
            if concepts.grouping(grouping) {
                Ok(())
            } else {
                Err(EngineError::UnknownConcept {
                    rule: rule.into(),
                    kind: "axioval:grouping".into(),
                    concept: grouping.clone(),
                })
            }
        }
        Selector::Property {
            property_set,
            property,
            value,
            ..
        } => {
            require_property(concepts, rule, property_set.as_deref(), property)?;
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

/// Every concept the ruleset may name: its packages' vocabulary and the
/// classifications, groupings and values it derives.
fn declared_concepts(
    ruleset: &RuleSetPackage,
    packages: &BTreeMap<&str, &DefinitionPackage>,
) -> Result<ConceptCatalog, EngineError> {
    let mut concepts = concept_catalog(ruleset, packages)?;
    concepts.declare_classifications(class_trees(ruleset.classifications.iter())?);
    concepts.declare_groupings(ruleset.groupings.keys().cloned());
    concepts.declare_values(ruleset.values.keys().cloned());
    Ok(concepts)
}

/// Adds `more` to the values merged from earlier rulesets; a name two
/// rulesets derive differently is refused.
fn merge_values(
    values: &mut BTreeMap<String, axioval_ir::contract::Expression>,
    more: &BTreeMap<String, axioval_ir::contract::Expression>,
) -> Result<(), EngineError> {
    for (name, expression) in more {
        match values.get(name) {
            Some(seen) if seen != expression => {
                return Err(EngineError::InvalidValue {
                    value: name.clone(),
                    detail: "two rulesets derive it differently".into(),
                });
            }
            _ => {
                values.insert(name.clone(), expression.clone());
            }
        }
    }
    Ok(())
}

/// Every property the ruleset's definition packages declare, by id.
fn vocabulary_properties<'a>(
    ruleset: &RuleSetPackage,
    packages: &BTreeMap<&str, &'a DefinitionPackage>,
) -> BTreeMap<&'a str, &'a axioval_ir::contract::PropertyDefinition> {
    ruleset
        .definition_packages
        .iter()
        .flat_map(|id| packages[id.as_str()].properties.iter())
        .map(|(id, property)| (id.as_str(), property))
        .collect()
}

/// Checks a rule's bound parameters: the concepts they name, and its
/// expressions' structure and types.
fn validate_bound(
    vocabulary: &crate::expression_binding::Vocabulary<'_>,
    rule: &str,
    parameters: &BTreeMap<String, ParameterValue>,
    definition: &RuleDefinition,
) -> Result<(), EngineError> {
    for value in parameters.values() {
        validate_parameter_concepts(vocabulary.concepts, rule, value)?;
    }
    crate::expression_binding::check_rule_expressions(
        vocabulary,
        rule,
        parameters,
        &definition.parameters,
    )
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
        } => require_property(concepts, rule, property_set.as_deref(), property),
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

/// Every rule in `folder` and its subfolders, each with the gates of the
/// folders around it, outermost first.
fn flatten<'a>(
    folder: &'a RuleFolder,
    outer: &[&'a RuleGate],
    out: &mut Vec<(&'a RuleInstance, Vec<&'a RuleGate>)>,
) {
    let mut gates = outer.to_vec();
    gates.extend(&folder.gate);
    out.extend(folder.rules.iter().map(|rule| (rule, gates.clone())));
    for child in &folder.folders {
        flatten(child, &gates, out);
    }
}

fn validate_signature(
    definition_id: &str,
    capability_id: &str,
    descriptors: &[ParameterDescriptor],
    parameters: &BTreeMap<String, axioval_ir::contract::ParameterDefinition>,
    authored: bool,
) -> Result<(), EngineError> {
    if authored {
        for (name, parameter) in parameters {
            if descriptors
                .iter()
                .any(|descriptor| descriptor.name == *name)
            {
                continue;
            }
            let scalar = matches!(
                parameter.kind,
                ParameterKind::String
                    | ParameterKind::Boolean
                    | ParameterKind::Integer
                    | ParameterKind::Number
                    | ParameterKind::Quantity
                    | ParameterKind::Enum
                    | ParameterKind::Date
                    | ParameterKind::DateTime
                    | ParameterKind::StringList
            );
            let table = parameter.kind == ParameterKind::Table;
            if !(scalar || table) {
                return contract_error(
                    definition_id,
                    capability_id,
                    &format!(
                        "authored parameter `{name}` must be a scalar value, a string list or a \
                         table"
                    ),
                );
            }
            let mut ids: Vec<_> = parameter.columns.iter().map(|column| &column.id).collect();
            ids.sort_unstable();
            ids.dedup();
            if table && (ids.is_empty() || ids.len() != parameter.columns.len()) {
                return contract_error(
                    definition_id,
                    capability_id,
                    &format!("table parameter `{name}` needs distinct columns"),
                );
            }
            if !table && !parameter.columns.is_empty() {
                return contract_error(
                    definition_id,
                    capability_id,
                    &format!("only a table parameter declares columns, not `{name}`"),
                );
            }
        }
    } else if descriptors.len() != parameters.len() {
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

/// A capability's table columns as `validate_row` reads them.
fn trusted_columns(columns: &[TableColumn]) -> Vec<(&str, ColumnKind, bool)> {
    columns
        .iter()
        .map(|column| (column.id, column.kind, column.required))
        .collect()
}

/// Checks one table row against columns: id, kind, whether required.
fn validate_row(columns: &[(&str, ColumnKind, bool)], row: &TableRow) -> Result<(), String> {
    for (id, cell) in row {
        let (_, kind, _) = columns
            .iter()
            .find(|(column, _, _)| column == id)
            .ok_or_else(|| format!("unknown column `{id}`"))?;
        if !cell_fits(*kind, cell) {
            return Err(format!("column `{id}` takes a {} cell", kind.as_str()));
        }
    }
    match columns
        .iter()
        .find(|(id, _, required)| *required && !row.contains_key(*id))
    {
        Some((id, _, _)) => Err(format!("required column `{id}` is empty")),
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
        ParameterKind::Expression => ParameterType::Expression,
        ParameterKind::StringList => ParameterType::StringList,
        ParameterKind::ReferenceList => ParameterType::ReferenceList,
        ParameterKind::Table => ParameterType::Table(&[]),
    }
}
