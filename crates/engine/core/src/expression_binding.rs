//! Checking a rule's expression parameters when the ruleset is compiled:
//! structure, the concepts they read and their types.

use std::collections::BTreeMap;

use axioval_ir::contract::{
    ColumnKind, Expression, ParameterDefinition, ParameterKind, ParameterValue, PropertyDefinition,
    PropertyValueKind, ValueDefinition,
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
        let ParameterValue::Expression { value: expression } = value else {
            continue;
        };
        let invalid = |path: String, detail: String| EngineError::InvalidExpression {
            rule: rule.into(),
            parameter: name.clone(),
            path,
            detail,
        };
        expression
            .validate()
            .map_err(|error| invalid(name.clone(), error.to_string()))?;
        check_as(expression, name, &Type::Boolean, &environment)
            .map_err(|error| invalid(error.path.clone(), error.to_string()))?;
    }
    Ok(())
}

/// What a ruleset's expressions may read: its concepts, the vocabulary's
/// property definitions and the types of its derived values.
pub(crate) struct Vocabulary<'a> {
    pub(crate) concepts: &'a ConceptCatalog,
    pub(crate) properties: &'a BTreeMap<&'a str, &'a PropertyDefinition>,
    pub(crate) values: BTreeMap<String, Type>,
}

/// Checks a ruleset's derived values in dependency order and gives their
/// expressions: each well formed, reading only declared concepts and
/// values, and of a type a property can state.
pub(crate) fn check_values<'a>(
    concepts: &'a ConceptCatalog,
    properties: &'a BTreeMap<&'a str, &'a PropertyDefinition>,
    values: &BTreeMap<String, ValueDefinition>,
) -> Result<(Vocabulary<'a>, BTreeMap<String, Expression>), EngineError> {
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
    Ok((
        Vocabulary {
            concepts,
            properties,
            values: types,
        },
        expressions,
    ))
}
