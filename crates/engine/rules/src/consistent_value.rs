//! Objects that agree on one property must agree on another.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, ParameterType, RuleCapability,
    RuleContext,
};
use axioval_ir::{Evidence, Object};

use crate::selection::select_objects;
use crate::support::{
    Parameters, Unavailable, display, finding, resolve, scope_key, undefined, value_key,
};

/// Requires objects sharing a `key` value to share their `value` too.
///
/// Doors of one type mark must have one fire rating; walls of one
/// construction type one thickness class. Objects are grouped by their key
/// value, compared without regard to case unless `case_sensitive` is `true`,
/// within one source (or the project with `across_sources`), of one kind
/// unless `same_kind` is `false`, and optionally within the objects a
/// declared `relationship` reaches, such as one storey.
///
/// A group whose members disagree raises one finding per member, naming the
/// members that hold another value. An absent value is a value of its own:
/// "some doors of this type state a rating and some do not" is a
/// disagreement. An object with no key value cannot be grouped and gets a
/// finding of its own.
pub struct ConsistentValue;

struct Member<'a> {
    object: &'a Object,
    value: String,
    shown: String,
    evidence: Vec<Evidence>,
}

impl RuleCapability for ConsistentValue {
    fn id(&self) -> &'static str {
        "axioval:capability.consistent-value"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("key", ParameterType::PropertyReference),
            ParameterDescriptor::required("value", ParameterType::PropertyReference),
            ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
            ParameterDescriptor::optional("same_kind", ParameterType::Boolean),
            ParameterDescriptor::optional("across_sources", ParameterType::Boolean),
        ]
        .into_iter()
        .chain(crate::support::traversal_parameters())
        .collect()
    }

    #[allow(clippy::too_many_lines)]
    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let parsed = (|| {
            Ok::<_, Unavailable>((
                parameters.required_property("key")?,
                parameters.required_property("value")?,
                parameters.boolean("case_sensitive")?.unwrap_or(false),
                parameters.boolean("same_kind")?.unwrap_or(true),
                parameters.boolean("across_sources")?.unwrap_or(false),
                parameters.traversal()?,
            ))
        })();
        let (key, value, case_sensitive, same_kind, across_sources, traversal) = match parsed {
            Ok(parsed) => parsed,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("consistent-value: {message}"),
                );
            }
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let mut groups: BTreeMap<(String, String, String), (String, Vec<Member<'_>>)> =
            BTreeMap::new();
        for object in selected {
            let answers = resolve(context, object, key)
                .and_then(|found_key| Ok((found_key, resolve(context, object, value)?)));
            let (found_key, found_value) = match answers {
                Ok(answers) => answers,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    continue;
                }
            };
            if undefined(found_key.value()) {
                evaluation.push_finding(finding(
                    rule,
                    &object.id,
                    format!("{key} has no value, so {value} cannot be compared"),
                    found_key.evidence(),
                    vec![],
                ));
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
            evidence.extend(found_key.evidence());
            evidence.extend(found_value.evidence());
            let kind = if same_kind {
                object.kind().to_ascii_lowercase()
            } else {
                String::new()
            };
            let key_value = found_key
                .value()
                .expect("an undefined key was handled above");
            let shown_value = display(found_value.value());
            let member = Member {
                object,
                value: found_value.value().map_or_else(
                    || "absent".to_owned(),
                    |held| value_key(held, true, case_sensitive),
                ),
                shown: shown_value,
                evidence,
            };
            groups
                .entry((scope, kind, value_key(key_value, true, case_sensitive)))
                .or_insert_with(|| (display(Some(key_value)), Vec::new()))
                .1
                .push(member);
        }
        for (shown_key, members) in groups.into_values() {
            let distinct: BTreeSet<&str> = members.iter().map(|m| m.value.as_str()).collect();
            if distinct.len() < 2 {
                continue;
            }
            for member in &members {
                let others: Vec<&Member<'_>> = members
                    .iter()
                    .filter(|other| other.value != member.value)
                    .collect();
                let mut seen = BTreeSet::new();
                let other_values: Vec<&str> = others
                    .iter()
                    .filter(|other| seen.insert(other.value.as_str()))
                    .map(|other| other.shown.as_str())
                    .collect();
                let mut evidence = member.evidence.clone();
                evidence.extend(
                    others
                        .iter()
                        .flat_map(|other| other.evidence.iter().cloned()),
                );
                evaluation.push_finding(finding(
                    rule,
                    &member.object.id,
                    format!(
                        "{value} is {} where other objects with {key} {shown_key} have {}",
                        member.shown,
                        other_values.join(", ")
                    ),
                    evidence,
                    others.iter().map(|other| other.object.id.clone()).collect(),
                ));
            }
        }
        evaluation
    }
}
