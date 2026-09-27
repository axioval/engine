//! Identifiers that must not repeat within a scope.

use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, ParameterType, RuleCapability,
    RuleContext,
};
use axioval_ir::{Evidence, Object, PropertyValue};

use crate::selection::select_objects;
use crate::support::{
    Parameters, PropertyRef, Unavailable, display, finding, resolve, scope_key, undefined,
    value_key,
};

/// Requires a property to be unique among the selected objects of one scope.
///
/// The typical use is a space number. Values are compared after trimming
/// (unless `trim` is `false`) and without regard to case (unless
/// `case_sensitive` is `true`). The scope is one source, since two models of
/// a federation number independently; `across_sources` widens it to the whole
/// project, and a declared `relationship` narrows it to the objects that
/// reach the same related objects, such as the spaces of one storey.
///
/// Numbers and quantities may be compared under a declared tolerance (see
/// `tolerance`, `relative_tolerance` and `decimals`), quantities only with
/// quantities of the same dimension. Rounding to `decimals` sorts values into
/// classes: objects whose values round alike are duplicates of each other.
/// A tolerance is not transitive, so it is judged pair by pair: an object is
/// a duplicate of every other object whose value lies within the tolerance
/// of its own, and its finding names exactly those; `1.0` and `1.2` under a
/// tolerance of `0.1` are both near `1.1` but not near each other.
///
/// Every object sharing a value gets one finding naming the others. An
/// object without a value (absent, null or blank) gets a finding of its own
/// unless `require_value` is `false`, in which case it is not compared.
pub struct UniqueValue;

/// An object holding a value: the object, the value as shown, its evidence.
type Holder<'a> = (&'a Object, String, Vec<Evidence>);

/// A number's comparison domain (unit-less, or one quantity dimension) and
/// its value, in SI for a quantity; `None` for anything that is not a number.
fn numeric(value: &PropertyValue) -> Option<Result<(String, f64), String>> {
    match value {
        PropertyValue::Decimal(value) => Some(Ok(("number".into(), *value))),
        PropertyValue::Integer(value) => Some(if value.unsigned_abs() <= 1 << 53 {
            #[allow(clippy::cast_precision_loss)]
            Ok(("number".into(), *value as f64))
        } else {
            Err("integer cannot be represented exactly as a decimal".into())
        }),
        PropertyValue::Quantity { value, dimension } => Some(Ok((
            format!("quantity:{}", dimension.unit_symbol()),
            *value,
        ))),
        _ => None,
    }
}

impl RuleCapability for UniqueValue {
    fn id(&self) -> &'static str {
        "axioval:capability.unique-value"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("trim", ParameterType::Boolean),
            ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
            ParameterDescriptor::optional("require_value", ParameterType::Boolean),
            ParameterDescriptor::optional("across_sources", ParameterType::Boolean),
        ]
        .into_iter()
        .chain(crate::support::traversal_parameters())
        .chain(crate::support::tolerance_parameters())
        .collect()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let parsed = (|| {
            Ok::<_, Unavailable>((
                parameters.required_property("property")?,
                parameters.boolean("trim")?.unwrap_or(true),
                parameters.boolean("case_sensitive")?.unwrap_or(false),
                parameters.boolean("require_value")?.unwrap_or(true),
                parameters.boolean("across_sources")?.unwrap_or(false),
                parameters.traversal()?,
                parameters.tolerance()?,
            ))
        })();
        let (property, trim, case_sensitive, require_value, across_sources, traversal, tolerance) =
            match parsed {
                Ok(parsed) => parsed,
                Err((reason, message)) => {
                    return CapabilityEvaluation::not_evaluated(
                        reason,
                        format!("unique-value: {message}"),
                    );
                }
            };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        // (scope, value) -> objects holding it, with their evidence.
        let mut holders: BTreeMap<(String, String), Vec<Holder<'_>>> = BTreeMap::new();
        // (scope, domain) -> numbers compared pair by pair under a tolerance.
        let mut near: BTreeMap<(String, String), Vec<(Holder<'_>, f64)>> = BTreeMap::new();
        for object in selected {
            let resolved = match resolve(context, object, property) {
                Ok(resolved) => resolved,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    continue;
                }
            };
            let value = resolved.value();
            if undefined(value) {
                if require_value {
                    evaluation.push_finding(finding(
                        rule,
                        &object.id,
                        format!("{property} has no value"),
                        resolved.evidence(),
                        vec![],
                    ));
                }
                continue;
            }
            let (scope, mut evidence) =
                match scope_key(context, traversal.as_ref(), across_sources, object) {
                    Ok(scope) => scope,
                    Err((reason, message)) => {
                        evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                        continue;
                    }
                };
            evidence.extend(resolved.evidence());
            let value = value.expect("an undefined value was handled above");
            let holder = (object, display(Some(value)), evidence);
            let number = if tolerance.is_exact() {
                None
            } else {
                numeric(value)
            };
            match number {
                None => holders
                    .entry((scope, value_key(value, trim, case_sensitive)))
                    .or_default()
                    .push(holder),
                Some(Err(message)) => evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    axioval_ir::NotEvaluatedReason::InvalidEvidence,
                    message,
                ),
                // Rounding is transitive: the rounded value is the key.
                Some(Ok((domain, number))) if tolerance.rounds() => holders
                    .entry((scope, format!("{domain}:{}", tolerance.round(number))))
                    .or_default()
                    .push(holder),
                Some(Ok((domain, number))) => {
                    near.entry((scope, domain))
                        .or_default()
                        .push((holder, number));
                }
            }
        }
        let suffix = tolerance.suffix();
        for group in holders.into_values() {
            report(
                rule,
                property,
                &suffix,
                &group,
                |_, _| true,
                &mut evaluation,
            );
        }
        for group in near.into_values() {
            let (group, numbers): (Vec<_>, Vec<_>) = group.into_iter().unzip();
            let near = |a: usize, b: usize| tolerance.equal(numbers[a], numbers[b]);
            report(rule, property, &suffix, &group, near, &mut evaluation);
        }
        evaluation
    }
}

/// Reports each holder that is `near` another one of its group, naming
/// exactly those: every other one for an equal key, and for a tolerance the
/// ones within it of this holder's own value.
fn report(
    rule: &CompiledRule,
    property: PropertyRef<'_>,
    suffix: &str,
    group: &[Holder<'_>],
    near: impl Fn(usize, usize) -> bool,
    evaluation: &mut CapabilityEvaluation,
) {
    for (index, (object, value, own)) in group.iter().enumerate() {
        let neighbours: Vec<&Holder<'_>> = group
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index && near(index, *other))
            .map(|(_, holder)| holder)
            .collect();
        if neighbours.is_empty() {
            continue;
        }
        let evidence = own
            .iter()
            .chain(neighbours.iter().flat_map(|(_, _, evidence)| evidence))
            .cloned()
            .collect();
        let related = std::iter::once(object.id.clone())
            .chain(neighbours.iter().map(|(other, _, _)| other.id.clone()))
            .collect();
        evaluation.push_finding(finding(
            rule,
            &object.id,
            format!(
                "{property} {value} is also used by {} other object(s){suffix}",
                neighbours.len()
            ),
            evidence,
            related,
        ));
    }
}
