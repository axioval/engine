//! Checking a rule's expression parameters when the ruleset is compiled:
//! structure, the concepts they read and their types.

use std::collections::BTreeMap;

use axioval_ir::contract::{
    ColumnKind, Expression, ParameterDefinition, ParameterKind, ParameterValue, PropertyDefinition,
    PropertyValueKind,
};

use axioval_ir::QuantityDimension;

use crate::EngineError;
use crate::concepts::ConceptCatalog;
use crate::expression::{Type, TypeEnvironment, Unit, check, check_as, measured_type, parse_unit};

/// The types an expression of one rule may read: the vocabulary's
/// properties, measured values, and the rule's own parameters.
struct RuleEnvironment<'a> {
    concepts: &'a ConceptCatalog,
    properties: &'a BTreeMap<&'a str, &'a PropertyDefinition>,
    values: &'a BTreeMap<String, Type>,
    rule: &'a str,
    parameters: &'a BTreeMap<String, ParameterValue>,
    declared: &'a BTreeMap<String, ParameterDefinition>,
}

impl TypeEnvironment for RuleEnvironment<'_> {
    fn property(&self, set: Option<&str>, name: &str) -> Result<Type, String> {
        if set == Some(axioval_ir::MEASURED_SET) {
            return measured_type(name);
        }
        if set == Some(axioval_ir::VALUE_SET) {
            return self.derived(name);
        }
        crate::compiler::require_property(self.concepts, self.rule, set, name)
            .map_err(|error| error.to_string())?;
        if set.is_some_and(axioval_ir::is_derived_set) {
            return Ok(Type::Any);
        }
        Ok(self.properties.get(name).map_or(Type::Any, |property| {
            property_type(&property.value_kind, property.unit_dimension.as_deref())
        }))
    }

    fn parameter(&self, name: &str) -> Result<Type, String> {
        let value = self
            .parameters
            .get(name)
            .ok_or_else(|| format!("the rule has no parameter `{name}`"))?;
        value_type(value).ok_or_else(|| format!("parameter `{name}` is no single value"))
    }

    fn derived(&self, name: &str) -> Result<Type, String> {
        self.values
            .get(name)
            .cloned()
            .ok_or_else(|| format!("the ruleset derives no value `{name}`"))
    }

    fn lookup(&self, table: &str, column: &str) -> Result<(BTreeMap<String, Type>, Type), String> {
        let declared = self
            .declared
            .get(table)
            .filter(|declared| declared.kind == ParameterKind::Table)
            .ok_or_else(|| format!("the rule has no table parameter `{table}`"))?;
        let columns: BTreeMap<String, Type> = declared
            .columns
            .iter()
            .map(|declared| {
                (
                    declared.id.clone(),
                    column_type(declared.kind, declared.unit_dimension.as_deref()),
                )
            })
            .collect();
        let result = columns
            .get(column)
            .cloned()
            .ok_or_else(|| format!("table `{table}` has no column `{column}`"))?;
        Ok((columns, result))
    }
}

/// The unit a declared `unitDimension` names (`length`, `area`, `volume`,
/// `plane_angle`, or a unit such as `W/m2K`), if it names one.
fn dimension_unit(dimension: Option<&str>) -> Option<Unit> {
    let dimension = dimension?.trim();
    let named = match dimension {
        "length" => Some(QuantityDimension::Length),
        "area" => Some(QuantityDimension::Area),
        "volume" => Some(QuantityDimension::Volume),
        "plane_angle" | "planeAngle" => Some(QuantityDimension::PlaneAngle),
        _ => None,
    };
    named.map_or_else(
        || parse_unit(dimension).ok().map(|(_, unit)| unit),
        |named| Some(Unit::of(Some(named))),
    )
}

/// A quantity of the declared dimension, or known only when read.
fn quantity(dimension: Option<&str>) -> Type {
    dimension_unit(dimension).map_or(Type::Any, Type::Number)
}

fn property_type(kind: &PropertyValueKind, dimension: Option<&str>) -> Type {
    match kind {
        PropertyValueKind::Quantity => quantity(dimension),
        PropertyValueKind::String => Type::Text,
        PropertyValueKind::Boolean => Type::Boolean,
        PropertyValueKind::Integer => Type::Integer,
        PropertyValueKind::Number => Type::NUMBER,
        PropertyValueKind::Enum => Type::Enum(None),
        PropertyValueKind::Date => Type::Date,
        PropertyValueKind::DateTime => Type::DateTime,
        // Every other kind is known when read.
        PropertyValueKind::Reference
        | PropertyValueKind::StringList
        | PropertyValueKind::ReferenceList => Type::Any,
    }
}

fn column_type(kind: ColumnKind, dimension: Option<&str>) -> Type {
    match kind {
        ColumnKind::Quantity => quantity(dimension),
        ColumnKind::String | ColumnKind::TextPattern => Type::Text,
        ColumnKind::Integer => Type::Integer,
        ColumnKind::Number => Type::NUMBER,
        ColumnKind::Boolean => Type::Boolean,
        ColumnKind::Date => Type::Date,
        ColumnKind::DateTime => Type::DateTime,
        ColumnKind::Selector | ColumnKind::Reference => Type::Any,
    }
}

/// The type of a scalar parameter value.
pub(crate) fn value_type(value: &ParameterValue) -> Option<Type> {
    Some(match value {
        ParameterValue::Boolean { .. } => Type::Boolean,
        ParameterValue::Integer { .. } => Type::Integer,
        ParameterValue::Number { .. } => Type::NUMBER,
        ParameterValue::Quantity { unit, .. } => {
            Type::Number(parse_unit(unit).map_or(Unit::NONE, |(_, unit)| unit))
        }
        ParameterValue::String { .. } => Type::Text,
        ParameterValue::Enum { value } => Type::Enum(Some(vec![value.clone()])),
        ParameterValue::Date { .. } => Type::Date,
        ParameterValue::DateTime { .. } => Type::DateTime,
        _ => return None,
    })
}

/// Checks every expression parameter of a bound rule: its structure, the
/// concepts it reads, and that its value is a truth.
pub(crate) fn check_rule_expressions(
    vocabulary: &Vocabulary<'_>,
    rule: &str,
    parameters: &BTreeMap<String, ParameterValue>,
    declared: &BTreeMap<String, ParameterDefinition>,
    descriptors: &[crate::ParameterDescriptor],
) -> Result<(), EngineError> {
    let environment = RuleEnvironment {
        concepts: vocabulary.concepts,
        properties: vocabulary.properties,
        values: &vocabulary.values,
        rule,
        parameters,
        declared,
    };
    for (name, value) in parameters {
        let parameter_type = descriptors
            .iter()
            .find(|descriptor| descriptor.name == *name)
            .map(|descriptor| descriptor.parameter_type);
        let invalid = |path: String, detail: String| EngineError::InvalidExpression {
            rule: rule.into(),
            parameter: name.clone(),
            path,
            detail,
        };
        let check = |expression: &Expression, path: &str, expected: Option<Type>| {
            expression
                .validate()
                .map_err(|error| invalid(path.to_owned(), error.to_string()))?;
            filter_concepts(vocabulary.concepts, rule, expression)?;
            let found = crate::expression::check(expression, path, &environment)
                .map_err(|error| invalid(error.path.clone(), error.to_string()))?;
            match expected {
                Some(expected) if !fits(&expected, &found) => Err(invalid(
                    path.to_owned(),
                    format!("`{path}`: {expected} is needed, not {found}"),
                )),
                _ => Ok(()),
            }
        };
        match (value, parameter_type) {
            (ParameterValue::Expression { value: expression }, kind) => {
                let expected = match kind {
                    None | Some(crate::ParameterType::Expression) => Some(Type::Boolean),
                    Some(kind) => parameter_type_of(kind),
                };
                check(expression, name, expected)?;
            }
            (ParameterValue::Table { value: rows }, Some(crate::ParameterType::Table(columns))) => {
                for (index, row) in rows.iter().enumerate() {
                    for (column, cell) in row {
                        let ParameterValue::Expression { value: expression } = cell else {
                            continue;
                        };
                        let expected = columns
                            .iter()
                            .find(|declared| declared.id == column)
                            .and_then(|declared| column_kind_type(declared.kind));
                        check(expression, &format!("{name}[{index}].{column}"), expected)?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Whether a value of type `found` may stand where `expected` is needed: a
/// quantity of any unit where a quantity is, text or an enumeration value
/// where either is, and a value known only when read anywhere.
fn fits(expected: &Type, found: &Type) -> bool {
    match (expected, found) {
        (_, Type::Any | Type::Null) | (Type::Text | Type::Enum(_), Type::Text | Type::Enum(_)) => {
            true
        }
        // A quantity parameter takes any unit but a plain number's.
        (Type::Number(unit), Type::Number(found)) if !unit.is_plain() => !found.is_plain(),
        (Type::Number(unit), Type::Integer) => unit.is_plain(),
        (expected, found) => expected == found,
    }
}

/// The type a computed parameter of `kind` must have.
fn parameter_type_of(kind: crate::ParameterType) -> Option<Type> {
    use crate::ParameterType as P;
    Some(match kind {
        P::Boolean => Type::Boolean,
        P::Integer => Type::Integer,
        P::Number => Type::NUMBER,
        // Any non-plain unit; `fits` accepts every one.
        P::Quantity => Type::Number(Unit::of(Some(QuantityDimension::Length))),
        P::String => Type::Text,
        P::Enum => Type::Enum(None),
        P::Date => Type::Date,
        P::DateTime => Type::DateTime,
        _ => return None,
    })
}

/// The type a computed cell of a column of `kind` must have.
fn column_kind_type(kind: ColumnKind) -> Option<Type> {
    Some(match kind {
        ColumnKind::String | ColumnKind::TextPattern => Type::Text,
        ColumnKind::Integer => Type::Integer,
        ColumnKind::Number => Type::NUMBER,
        ColumnKind::Quantity => Type::Number(Unit::of(Some(QuantityDimension::Length))),
        ColumnKind::Boolean => Type::Boolean,
        ColumnKind::Date => Type::Date,
        ColumnKind::DateTime => Type::DateTime,
        ColumnKind::Selector | ColumnKind::Reference => return None,
    })
}

/// What a ruleset's expressions may read: its concepts, the vocabulary's
/// property definitions and the types of its derived values.
pub(crate) struct Vocabulary<'a> {
    pub(crate) concepts: &'a ConceptCatalog,
    pub(crate) properties: &'a BTreeMap<&'a str, &'a PropertyDefinition>,
    pub(crate) values: BTreeMap<String, Type>,
}

/// Checks a ruleset's derived values in dependency order, then every
/// expression selector it holds, and gives the values' expressions: each
/// well formed, reading only declared concepts and values.
pub(crate) fn check_values<'a>(
    concepts: &'a ConceptCatalog,
    properties: &'a BTreeMap<&'a str, &'a PropertyDefinition>,
    ruleset: &axioval_ir::RuleSetPackage,
) -> Result<(Vocabulary<'a>, BTreeMap<String, Expression>), EngineError> {
    let values = &ruleset.values;
    let mut types = BTreeMap::new();
    let none = BTreeMap::new();
    let undeclared = BTreeMap::new();
    for name in crate::values::order(values)? {
        let expression = &values[&name].expression;
        let invalid = |detail: String| EngineError::InvalidValue {
            value: name.clone(),
            detail,
        };
        expression
            .validate()
            .map_err(|error| invalid(error.to_string()))?;
        filter_concepts(concepts, &name, expression)?;
        let environment = RuleEnvironment {
            concepts,
            properties,
            values: &types,
            rule: &name,
            parameters: &none,
            declared: &undeclared,
        };
        let found = check(expression, &format!("values.{name}"), &environment)
            .map_err(|error| invalid(error.to_string()))?;
        types.insert(name, found);
    }
    let expressions = values
        .iter()
        .map(|(name, definition)| (name.clone(), definition.expression.clone()))
        .collect();
    let vocabulary = Vocabulary {
        concepts,
        properties,
        values: types,
    };
    check_ruleset_selectors(&vocabulary, ruleset)?;
    Ok((vocabulary, expressions))
}

/// Checks an expression's structure and that every property it reads is a
/// declared concept, a measured name the registry accepts or a value the
/// ruleset derives; its types are checked once the values are known.
pub(crate) fn expression_concepts(
    concepts: &ConceptCatalog,
    rule: &str,
    expression: &Expression,
) -> Result<(), EngineError> {
    let invalid = |detail: String| EngineError::InvalidExpression {
        rule: rule.into(),
        parameter: "selector".into(),
        path: "selector.expression".into(),
        detail,
    };
    expression
        .validate()
        .map_err(|error| invalid(error.to_string()))?;
    let mut pending = vec![expression];
    while let Some(node) = pending.pop() {
        if let Expression::Property {
            property_set,
            property,
            ..
        } = node
        {
            crate::compiler::require_property(concepts, rule, property_set.as_deref(), property)?;
        }
        pending.extend(node.children());
    }
    filter_concepts(concepts, rule, expression)
}

/// Checks the concepts every aggregate member filter in `expression` names.
fn filter_concepts(
    concepts: &ConceptCatalog,
    rule: &str,
    expression: &Expression,
) -> Result<(), EngineError> {
    expression
        .filters()
        .into_iter()
        .try_for_each(|filter| crate::compiler::validate_selector_concepts(concepts, rule, filter))
}

/// Checks that every expression of `selector` is a truth over what the
/// ruleset declares, reading no rule parameter. `owner` and `place` name
/// where the selector sits (a rule and its applicability, a classification
/// and its row).
pub(crate) fn check_selector(
    vocabulary: &Vocabulary<'_>,
    owner: &str,
    place: &str,
    selector: &axioval_ir::contract::Selector,
) -> Result<(), EngineError> {
    let none = BTreeMap::new();
    let undeclared = BTreeMap::new();
    let environment = RuleEnvironment {
        concepts: vocabulary.concepts,
        properties: vocabulary.properties,
        values: &vocabulary.values,
        rule: owner,
        parameters: &none,
        declared: &undeclared,
    };
    for expression in selector.expressions() {
        check_as(
            expression,
            "selector.expression",
            &Type::Boolean,
            &environment,
        )
        .map_err(|error| EngineError::InvalidExpression {
            rule: owner.into(),
            parameter: place.into(),
            path: error.path.clone(),
            detail: error.to_string(),
        })?;
    }
    Ok(())
}

/// Checks every expression selector of `ruleset`: in its rules'
/// applicability and selector parameters (table cells included), its
/// classifications' rows, its groupings' members and its relations' ends.
pub(crate) fn check_ruleset_selectors(
    vocabulary: &Vocabulary<'_>,
    ruleset: &axioval_ir::RuleSetPackage,
) -> Result<(), EngineError> {
    use axioval_ir::contract::{RuleApplicability, RuleFolder, Selector};
    fn values<'a>(value: &'a ParameterValue, out: &mut Vec<&'a Selector>) {
        match value {
            ParameterValue::Selector { value } => out.push(value),
            ParameterValue::Table { value: rows } => {
                for cell in rows.iter().flat_map(std::collections::BTreeMap::values) {
                    values(cell, out);
                }
            }
            _ => {}
        }
    }
    fn folder(vocabulary: &Vocabulary<'_>, folder_: &RuleFolder) -> Result<(), EngineError> {
        for rule in &folder_.rules {
            let applicability: Vec<&Selector> = match &rule.applicability {
                RuleApplicability::Selector(selector) => vec![selector],
                RuleApplicability::Groups(groups) => groups
                    .groups
                    .values()
                    .map(|group| &group.selector)
                    .collect(),
            };
            for selector in applicability {
                check_selector(vocabulary, &rule.id, "applicability", selector)?;
            }
            for (name, value) in &rule.parameters {
                let mut selectors = Vec::new();
                values(value, &mut selectors);
                for selector in selectors {
                    check_selector(vocabulary, &rule.id, name, selector)?;
                }
            }
        }
        folder_
            .folders
            .iter()
            .try_for_each(|inner| folder(vocabulary, inner))
    }
    folder(vocabulary, &ruleset.root)?;
    for (id, classification) in &ruleset.classifications {
        for (index, row) in classification.rows.iter().enumerate() {
            check_selector(vocabulary, id, &format!("rows[{index}]"), &row.selector)?;
        }
    }
    for (id, grouping) in &ruleset.groupings {
        check_selector(vocabulary, id, "members", &grouping.members)?;
    }
    for (id, relation) in &ruleset.relations {
        check_selector(vocabulary, id, "from", &relation.from)?;
        check_selector(vocabulary, id, "to", &relation.to)?;
    }
    Ok(())
}
