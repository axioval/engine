//! Identifiers that must not repeat within a scope.

use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, ParameterType, RuleCapability,
    RuleContext,
};
use axioval_ir::{Evidence, Object};

use crate::selection::select_objects;
use crate::support::{
    Parameters, Unavailable, display, finding, resolve, scope_key, undefined, value_key,
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
/// Every object sharing a value gets one finding naming the others. An
/// object without a value (absent, null or blank) gets a finding of its own
/// unless `require_value` is `false`, in which case it is not compared.
pub struct UniqueValue;

/// An object holding a value: the object, the value as shown, its evidence.
type Holder<'a> = (&'a Object, String, Vec<Evidence>);

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
            ))
        })();
        let (property, trim, case_sensitive, require_value, across_sources, traversal) =
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
            holders
                .entry((scope, value_key(value, trim, case_sensitive)))
                .or_default()
                .push((object, display(Some(value)), evidence));
        }
        for group in holders.into_values().filter(|group| group.len() > 1) {
            let others = group.len() - 1;
            for (object, value, _) in &group {
                let evidence = group
                    .iter()
                    .flat_map(|(_, _, evidence)| evidence.iter().cloned())
                    .collect();
                let related = group.iter().map(|(other, _, _)| other.id.clone()).collect();
                evaluation.push_finding(finding(
                    rule,
                    &object.id,
                    format!("{property} {value} is also used by {others} other object(s)"),
                    evidence,
                    related,
                ));
            }
        }
        evaluation
    }
}
