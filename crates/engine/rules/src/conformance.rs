//! Every selected object must satisfy a declared requirement selector.

use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, ParameterType, RuleCapability,
    RuleContext,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, ObjectId};

use crate::selection::{Selection, select_objects, selector_matches};
use crate::support::{Parameters, PropertyRef, display, finding, resolve, undefined, value_key};

/// Checks each selected object against a `requirement` selector.
///
/// This is the shape of every "agreed list" check. Each allowed combination
/// of values is one `allOf` of property conditions, and the agreed list is
/// their `anyOf`: a space passes when its type, name and number together
/// match at least one agreed row, a door when its construction type is one
/// of the types agreed for doors. An object the requirement cannot be
/// decided for (a property the source cannot resolve) is not evaluated; it
/// is never read as conforming or as violating.
///
/// A failing object is one of two results. When none of the properties the
/// requirement consults has a value (each is absent, null or blank), the
/// object gets its own "no value" finding naming them. Otherwise its values
/// are unknown to the list: objects holding the same combination of values
/// (an empty one shown as absent, null or blank) share one finding, against
/// the first of them, naming the values and relating the others. A
/// requirement that consults no property reports each object on its own.
///
/// The finding cites every property fact the requirement consulted, so the
/// reviewer sees the values that failed. `message` replaces the default
/// text of the unknown-value finding and is followed by the values.
pub struct SelectorConformance;

/// Objects holding one unknown combination of values.
struct Unknown {
    shown: String,
    objects: Vec<ObjectId>,
    evidence: Vec<Evidence>,
}

const DEFAULT_MESSAGE: &str = "does not match any agreed combination of values";

/// The key part of an absent, null or blank value. Real values carry a
/// `text:` or `value:` prefix, so this cannot collide with one.
const NO_VALUE: &str = "no value";

impl RuleCapability for SelectorConformance {
    fn id(&self) -> &'static str {
        "axioval:capability.selector-conformance"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("requirement", ParameterType::Selector),
            ParameterDescriptor::optional("message", ParameterType::String),
        ]
    }

    #[allow(clippy::too_many_lines)]
    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let parsed = parameters
            .required_selector("requirement")
            .and_then(|requirement| Ok((requirement, parameters.string("message")?)));
        let (requirement, message) = match parsed {
            Ok(parsed) => parsed,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("selector-conformance: {message}"),
                );
            }
        };
        let message = message.unwrap_or(DEFAULT_MESSAGE);
        let mut consulted = Vec::new();
        consulted_properties(requirement, &mut consulted);
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let mut unknown: BTreeMap<Vec<String>, Unknown> = BTreeMap::new();
        for object in selected {
            let mut evidence = Vec::new();
            match selector_matches(context, requirement, object, &mut evidence) {
                Selection::Match => {}
                Selection::NotEvaluated(reason, why) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, why);
                }
                Selection::NoMatch => {
                    let answers: Result<Vec<_>, _> = consulted
                        .iter()
                        .map(|property| resolve(context, object, *property))
                        .collect();
                    let answers = match answers {
                        Ok(answers) if !answers.is_empty() => answers,
                        // Nothing to name or group by: report the object alone.
                        _ => {
                            evaluation.push_finding(finding(
                                rule,
                                &object.id,
                                message.to_owned(),
                                evidence,
                                vec![],
                            ));
                            continue;
                        }
                    };
                    for answer in &answers {
                        evidence.extend(answer.evidence());
                    }
                    if answers.iter().all(|answer| undefined(answer.value())) {
                        let names: Vec<String> =
                            consulted.iter().map(ToString::to_string).collect();
                        evaluation.push_finding(finding(
                            rule,
                            &object.id,
                            format!(
                                "{} has no value to compare with the agreed list",
                                names.join(", ")
                            ),
                            evidence,
                            vec![],
                        ));
                        continue;
                    }
                    let key = answers
                        .iter()
                        .map(|answer| match answer.value() {
                            Some(value) if !undefined(Some(value)) => value_key(value, false, true),
                            _ => NO_VALUE.to_owned(),
                        })
                        .collect();
                    let group = unknown.entry(key).or_insert_with(|| Unknown {
                        shown: consulted
                            .iter()
                            .zip(&answers)
                            .map(|(property, answer)| {
                                format!("{property} {}", display(answer.value()))
                            })
                            .collect::<Vec<_>>()
                            .join(", "),
                        objects: Vec::new(),
                        evidence: Vec::new(),
                    });
                    group.objects.push(object.id.clone());
                    group.evidence.extend(evidence);
                }
            }
        }
        for group in unknown.into_values() {
            let mut objects = group.objects.into_iter();
            if let Some(subject) = objects.next() {
                evaluation.push_finding(finding(
                    rule,
                    &subject,
                    format!("{message}: {}", group.shown),
                    group.evidence,
                    objects.collect(),
                ));
            }
        }
        evaluation
    }
}

/// The distinct properties `selector` consults, in declaration order.
fn consulted_properties<'a>(selector: &'a Selector, found: &mut Vec<PropertyRef<'a>>) {
    match selector {
        Selector::Property {
            property_set,
            property,
            ..
        } => {
            let set = property_set.as_deref();
            if !found
                .iter()
                .any(|seen| seen.set == set && seen.name == property)
            {
                found.push(PropertyRef {
                    set,
                    name: property,
                });
            }
        }
        Selector::AllOf { operands } | Selector::AnyOf { operands } => {
            for operand in operands {
                consulted_properties(operand, found);
            }
        }
        Selector::Not { operand } => consulted_properties(operand, found),
        // A related selector consults the properties of other objects, not
        // of the one being judged.
        Selector::All
        | Selector::EntityType { .. }
        | Selector::Classification { .. }
        | Selector::Discipline { .. }
        | Selector::Related { .. } => {}
    }
}
