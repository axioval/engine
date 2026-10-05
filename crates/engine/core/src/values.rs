//! Values a ruleset derives per object from expressions
//! ([`axioval_ir::contract::ValueDefinition`]), answered as the reserved set
//! [`axioval_ir::VALUE_SET`].
//!
//! Each value is evaluated at most once per object in a run and cached. It
//! reads properties through the run's own resolver, so a value may read
//! stated, measured, classified and other derived values; compilation
//! refuses a cycle. Its evidence cites every read it was computed from.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use axioval_ir::contract::{Expression, ValueDefinition};
use axioval_ir::{Evidence, ObjectId, Property, PropertyValue, VALUE_SET};

use crate::EngineError;
use crate::concepts::ConceptBindings;
use crate::expression::{ExpressionContext, Leaf, Value, evaluate};
use crate::properties::{
    CompletePropertyAbsenceEvidence, PropertyRequest, PropertyResolution, PropertyResolutionError,
    ResolvedProperty,
};

/// The names of the derived values `expression` reads, as `derived` nodes
/// or properties of [`VALUE_SET`].
#[must_use]
pub(crate) fn references(expression: &Expression) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut pending = vec![expression];
    while let Some(node) = pending.pop() {
        match node {
            Expression::Derived { name, .. } => {
                names.insert(name.clone());
            }
            Expression::Property {
                property_set: Some(set),
                property,
                ..
            } if set == VALUE_SET => {
                names.insert(property.clone());
            }
            _ => {}
        }
        pending.extend(node.children());
        if let Expression::Aggregate {
            filter: Some(filter),
            ..
        } = node
        {
            pending.extend(filter.expressions());
        }
    }
    names
}

/// The ruleset's values in an order where each follows the values it reads.
///
/// # Errors
///
/// [`EngineError::InvalidValue`] for a value reading an undeclared value,
/// or for a cycle, naming it (`a → b → a`).
pub(crate) fn order(
    values: &BTreeMap<String, ValueDefinition>,
) -> Result<Vec<String>, EngineError> {
    fn visit(
        name: &str,
        values: &BTreeMap<String, ValueDefinition>,
        path: &mut Vec<String>,
        done: &mut BTreeSet<String>,
        ordered: &mut Vec<String>,
    ) -> Result<(), EngineError> {
        if done.contains(name) {
            return Ok(());
        }
        if let Some(start) = path.iter().position(|seen| seen == name) {
            let mut cycle: Vec<&str> = path[start..].iter().map(String::as_str).collect();
            cycle.push(name);
            return Err(EngineError::InvalidValue {
                value: path[start].clone(),
                detail: format!("the values read one another: {}", cycle.join(" → ")),
            });
        }
        let definition = &values[name];
        path.push(name.to_owned());
        for read in references(&definition.expression) {
            if !values.contains_key(&read) {
                return Err(EngineError::InvalidValue {
                    value: name.to_owned(),
                    detail: format!("it reads `{read}`, which the ruleset does not declare"),
                });
            }
            visit(&read, values, path, done, ordered)?;
        }
        path.pop();
        done.insert(name.to_owned());
        ordered.push(name.to_owned());
        Ok(())
    }
    let mut ordered = Vec::new();
    let mut done = BTreeSet::new();
    for name in values.keys() {
        visit(name, values, &mut Vec::new(), &mut done, &mut ordered)?;
    }
    Ok(ordered)
}

/// How a value reads a property: the run's resolver.
pub(crate) type Resolver<'a> =
    dyn Fn(&PropertyRequest) -> Result<PropertyResolution, PropertyResolutionError> + 'a;

/// A value computed for one object: the value and the evidence it rests
/// on, or why it cannot be computed.
pub(crate) type Computed = Result<(Value, Vec<Evidence>), String>;

/// The run's derived values and their per-object cache.
pub(crate) struct DerivedValues {
    definitions: Arc<BTreeMap<String, Expression>>,
    bindings: Option<ConceptBindings>,
    budget: Option<Arc<crate::expression::EvaluationBudget>>,
    types: Option<Arc<crate::expression::DeclaredTypes>>,
    cache: Mutex<BTreeMap<(ObjectId, String), Computed>>,
}

/// The value expressions of a plan, installed in a run's services.
#[derive(Clone, Debug, Default)]
pub(crate) struct ValueExpressions(pub(crate) Arc<BTreeMap<String, Expression>>);

impl DerivedValues {
    /// Values of `definitions`, binding concepts through `bindings` and
    /// typed as `types` declares, with an empty cache.
    pub(crate) fn new(
        definitions: Arc<BTreeMap<String, Expression>>,
        bindings: Option<ConceptBindings>,
        budget: Option<Arc<crate::expression::EvaluationBudget>>,
        types: Option<Arc<crate::expression::DeclaredTypes>>,
    ) -> Self {
        Self {
            definitions,
            bindings,
            budget,
            types,
            cache: Mutex::new(BTreeMap::new()),
        }
    }

    /// The value `name` of `object`, reading through `resolver`, computed
    /// once per run.
    pub(crate) fn compute(
        &self,
        object: &ObjectId,
        name: &str,
        resolver: &Resolver<'_>,
    ) -> Computed {
        let key = (object.clone(), name.to_owned());
        if let Some(cached) = self
            .cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(&key).cloned())
        {
            return cached;
        }
        let computed = match self.definitions.get(name) {
            None => Err(format!("the ruleset declares no value `{name}`")),
            Some(expression) => {
                let mut leaves = ObjectReads {
                    values: self,
                    object,
                    resolver,
                };
                let evaluation = evaluate(expression, &format!("values.{name}"), &mut leaves);
                let evidence = evaluation
                    .reads
                    .iter()
                    .flat_map(|read| read.leaf.evidence.iter().cloned())
                    .collect();
                evaluation
                    .outcome
                    .map(|value| (value, evidence))
                    .map_err(|why| why.to_string())
            }
        };
        // Held only to store: a value reading another computes it first.
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(key, computed.clone());
        }
        computed
    }

    /// Answers a request in [`VALUE_SET`].
    pub(crate) fn resolve(
        &self,
        request: &PropertyRequest,
        resolver: &Resolver<'_>,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        let object = request.object_id();
        let name = request.property();
        let (value, read) = self.compute(object, name, resolver).map_err(|why| {
            PropertyResolutionError::Incomplete(format!(
                "`{VALUE_SET}` value `{name}` of {object}: {why}"
            ))
        })?;
        let mut locators: Vec<String> = read
            .iter()
            .map(|evidence| evidence.locator.clone())
            .collect();
        locators.sort();
        locators.dedup();
        let locator = format!("{VALUE_SET}/{name}: {}", locators.join("; "));
        let mut evidence = Evidence::exact(object.source.clone(), locator);
        // A number is exact only where everything it was computed from is:
        // a point computed from an approximation is as approximate as an
        // interval, never inferred exact from its being a point.
        let measured_exactly = read.iter().all(|evidence| evidence.exact);
        let value = match property_value(&value, measured_exactly) {
            Ok(Some(value)) => value,
            Ok(None) => {
                return Ok(PropertyResolution::Absent(
                    CompletePropertyAbsenceEvidence::try_new(request.clone(), evidence)?,
                ));
            }
            Err(why) => {
                return Err(PropertyResolutionError::Incomplete(format!(
                    "`{VALUE_SET}` value `{name}` of {object}: {why}"
                )));
            }
        };
        // Exact when the value is certain: an interval, or a point computed
        // from an approximation, never is.
        evidence.exact = !matches!(value, PropertyValue::Measured { .. });
        let property = Property::new(VALUE_SET, name, value)
            .map_err(|_| PropertyResolutionError::InvalidRequest)?
            .with_evidence(evidence);
        Ok(PropertyResolution::Present(ResolvedProperty::try_new(
            request.clone(),
            property,
        )?))
    }
}

/// A computed value as a property states it; `None` for `null`. A number
/// is a quantity or a decimal only where it is a point computed exactly.
fn property_value(value: &Value, exact: bool) -> Result<Option<PropertyValue>, String> {
    Ok(Some(match value {
        Value::Null => return Ok(None),
        Value::Boolean(value) => PropertyValue::Boolean(*value),
        Value::Text(value) | Value::Enum(value) => PropertyValue::String(value.clone()),
        Value::Date(value) => PropertyValue::Date(*value),
        Value::DateTime(value) => PropertyValue::DateTime(*value),
        Value::Number { value, unit } => match unit.dimension()? {
            Some(dimension) if exact && value.is_point() => PropertyValue::Quantity {
                value: value.lower,
                dimension,
            },
            None if exact && value.is_point() => PropertyValue::Decimal(value.lower),
            // A number known only to an interval, or approximately.
            dimension => PropertyValue::Measured {
                lower: value.lower,
                upper: value.upper,
                dimension,
            },
        },
    }))
}

/// Answers a value's leaves for one object through the run's resolver.
struct ObjectReads<'a> {
    values: &'a DerivedValues,
    object: &'a ObjectId,
    resolver: &'a Resolver<'a>,
}

impl ExpressionContext for ObjectReads<'_> {
    fn spend(&mut self) -> bool {
        self.values
            .budget
            .as_ref()
            .is_none_or(|budget| budget.spend())
    }

    fn property(&mut self, set: Option<&str>, name: &str) -> Leaf {
        if set == Some(VALUE_SET) {
            return self.derived(name);
        }
        let request = match crate::concepts::bind_request(
            self.values.bindings.as_ref(),
            self.object,
            set,
            name,
        ) {
            Ok(request) => request,
            Err((_, why)) => return Leaf::unreadable(why),
        };
        match (self.resolver)(&request) {
            Ok(PropertyResolution::Present(resolved)) => Leaf {
                value: Value::from_property(&resolved.property().value),
                evidence: resolved.property().evidence.iter().cloned().collect(),
            },
            Ok(PropertyResolution::Absent(proof)) => Leaf {
                value: Ok(Value::Null),
                evidence: vec![proof.evidence().clone()],
            },
            Err(error) => Leaf::unreadable(error.to_string()),
        }
    }

    fn parameter(&mut self, name: &str) -> Leaf {
        Leaf::unreadable(format!("a value reads no rule parameter, not `{name}`"))
    }

    fn declared_types(&self) -> Option<&crate::expression::DeclaredTypes> {
        self.values.types.as_deref()
    }

    fn derived(&mut self, name: &str) -> Leaf {
        match self.values.compute(self.object, name, self.resolver) {
            Ok((value, evidence)) => Leaf {
                value: Ok(value),
                evidence,
            },
            Err(why) => Leaf::unreadable(why),
        }
    }
}

/// The derived value `name` of `object` in the run whose services these
/// are, as an expression reads it: the value with its evidence, computed at
/// most once per run.
pub fn derived_value(services: &crate::ServiceRegistry, object: &ObjectId, name: &str) -> Leaf {
    let (Some(values), Some(resolver)) = (
        services.get::<Arc<DerivedValues>>(),
        services.get::<crate::PropertyResolutionServiceHandle>(),
    ) else {
        return Leaf::unreadable(format!("the run derives no value `{name}`"));
    };
    match values.compute(object, name, &|request: &PropertyRequest| {
        resolver.resolve(request)
    }) {
        Ok((value, evidence)) => Leaf {
            value: Ok(value),
            evidence,
        },
        Err(why) => Leaf::unreadable(why),
    }
}
