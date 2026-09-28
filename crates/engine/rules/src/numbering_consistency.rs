//! Numbers read from a pattern that must agree within a scope.

use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::{Evidence, Object, PropertyValue};
use regex::Regex;

use crate::selection::select_objects;
use crate::support::{
    Parameters, PropertyRef, Unavailable, display, finding, invalid, resolve, scope_key, undefined,
};
use crate::xsd_pattern;

/// Requires the numbers of one scope to share a prefix and leave no gaps.
///
/// The typical use is the space numbers of one storey: `B-101`, `B-102`,
/// `B-104` read through the pattern `B-(\d+)` give 101, 102 and 104. The
/// `pattern` is an XML Schema pattern over the whole value with exactly one
/// group, which must capture ASCII digits. Scopes are formed as in
/// `unique-value`: one source, the project with `across_sources`, or the
/// objects reaching the same related objects through a declared traversal.
///
/// With `prefix_length`, the first that many digits must be the same across
/// the scope; the objects departing from the prefix most objects share are
/// reported, and when no prefix predominates every object of the scope is.
/// With `gap_free`, the distinct numbers, sorted, must step by one; the
/// objects above each gap are reported. At least one check must be declared.
///
/// A value that is absent, blank, not text or a whole number, or does not
/// match the pattern is not a number: its object is not evaluated, never
/// passed. An object whose value or scope cannot be read might fill a gap or
/// shift the predominant prefix, so a gap it could fill is not evaluated
/// rather than reported.
pub struct NumberingConsistency;

impl RuleCapability for NumberingConsistency {
    fn id(&self) -> &'static str {
        "axioval:capability.numbering-consistency"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("property", ParameterType::PropertyReference),
            ParameterDescriptor::required("pattern", ParameterType::String),
            ParameterDescriptor::optional("prefix_length", ParameterType::Integer),
            ParameterDescriptor::optional("gap_free", ParameterType::Boolean),
            ParameterDescriptor::optional("across_sources", ParameterType::Boolean),
        ]
        .into_iter()
        .chain(crate::support::traversal_parameters())
        .collect()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match Config::read(&Parameters(rule)) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("numbering-consistency: {message}"),
                );
            }
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let mut scopes: BTreeMap<String, Scope<'_>> = BTreeMap::new();
        // Objects in no known scope, by source key, with their number if read.
        let mut stray: Vec<(String, Option<u64>)> = Vec::new();
        for object in selected {
            let source = if config.across_sources {
                String::new()
            } else {
                object.id.source.to_string()
            };
            let scope = |context: &RuleContext<'_>| {
                scope_key(
                    context,
                    config.traversal.as_ref(),
                    config.across_sources,
                    object,
                )
            };
            let resolved = match resolve(context, object, config.property) {
                Ok(resolved) => resolved,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    match scope(context) {
                        Ok((key, _)) => {
                            scopes
                                .entry(key)
                                .or_insert_with(|| Scope::new(source))
                                .undecided += 1;
                        }
                        Err(_) => stray.push((source, None)),
                    }
                    continue;
                }
            };
            let value = resolved.value();
            let digits = match config.digits(value) {
                Ok(digits) => digits,
                Err(message) => {
                    evaluation.push_object_not_evaluated(
                        object.id.clone(),
                        NotEvaluatedReason::IncompleteEvidence,
                        format!("{} {} {message}", config.property, display(value)),
                    );
                    continue;
                }
            };
            let Ok(number) = digits.parse::<u64>() else {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{} {} holds a number too large to compare",
                        config.property,
                        display(value)
                    ),
                );
                continue;
            };
            let (key, mut evidence) = match scope(context) {
                Ok(scope) => scope,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    stray.push((source, Some(number)));
                    continue;
                }
            };
            evidence.extend(resolved.evidence());
            scopes
                .entry(key)
                .or_insert_with(|| Scope::new(source))
                .members
                .push(Member {
                    object,
                    shown: display(value),
                    digits,
                    number,
                    evidence,
                });
        }
        judge(rule, &config, &scopes, &stray, &mut evaluation);
        evaluation
    }
}

/// Runs the declared checks on every scope.
fn judge(
    rule: &CompiledRule,
    config: &Config<'_>,
    scopes: &BTreeMap<String, Scope<'_>>,
    stray: &[(String, Option<u64>)],
    evaluation: &mut CapabilityEvaluation,
) {
    for scope in scopes.values() {
        let strays: Vec<Option<u64>> = stray
            .iter()
            .filter(|(source, _)| *source == scope.source)
            .map(|(_, number)| *number)
            .collect();
        if let Some(length) = config.prefix_length {
            check_prefix(rule, config, scope, length, strays.len(), evaluation);
        }
        if config.gap_free {
            check_gaps(rule, config, scope, &strays, evaluation);
        }
    }
}

struct Config<'a> {
    property: PropertyRef<'a>,
    pattern: Regex,
    prefix_length: Option<usize>,
    gap_free: bool,
    across_sources: bool,
    traversal: Option<crate::support::Traversal>,
}

impl<'a> Config<'a> {
    fn read(parameters: &Parameters<'a>) -> Result<Self, Unavailable> {
        let source = parameters.required_string("pattern")?;
        let pattern = xsd_pattern::compile(source)
            .map_err(|error| invalid(format!("pattern {source:?}: {error}")))?;
        if pattern.captures_len() != 2 {
            return Err(invalid(format!(
                "pattern {source:?} must have exactly one group, the number"
            )));
        }
        let prefix_length = match parameters.integer("prefix_length")? {
            None => None,
            Some(length) => Some(
                usize::try_from(length)
                    .ok()
                    .filter(|length| *length > 0)
                    .ok_or_else(|| invalid("prefix_length must be positive"))?,
            ),
        };
        let gap_free = parameters.boolean("gap_free")?.unwrap_or(false);
        if prefix_length.is_none() && !gap_free {
            return Err(invalid(
                "declare `prefix_length`, `gap_free` or both; nothing is checked otherwise",
            ));
        }
        Ok(Self {
            property: parameters.required_property("property")?,
            pattern,
            prefix_length,
            gap_free,
            across_sources: parameters.boolean("across_sources")?.unwrap_or(false),
            traversal: parameters.traversal()?,
        })
    }

    /// The digits the pattern captures from a value, or why there are none.
    fn digits(&self, value: Option<&PropertyValue>) -> Result<String, &'static str> {
        if undefined(value) {
            return Err("is not set, so it has no number");
        }
        let text = match value {
            Some(PropertyValue::String(text)) => text.clone(),
            Some(PropertyValue::Integer(number)) => number.to_string(),
            _ => return Err("is neither text nor a whole number"),
        };
        let captured = self
            .pattern
            .captures(&text)
            .ok_or("does not match the pattern")?
            .get(1)
            .ok_or("does not capture a number")?
            .as_str();
        if captured.is_empty() || !captured.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("captures something other than digits");
        }
        Ok(captured.to_owned())
    }
}

struct Member<'a> {
    object: &'a Object,
    shown: String,
    digits: String,
    number: u64,
    evidence: Vec<Evidence>,
}

struct Scope<'a> {
    source: String,
    members: Vec<Member<'a>>,
    /// Objects of this scope whose number could not be read.
    undecided: usize,
}

impl Scope<'_> {
    fn new(source: String) -> Self {
        Self {
            source,
            members: Vec::new(),
            undecided: 0,
        }
    }
}

fn report(
    rule: &CompiledRule,
    member: &Member<'_>,
    message: String,
    related: &[&Member<'_>],
) -> axioval_ir::Finding {
    let mut evidence = member.evidence.clone();
    evidence.extend(
        related
            .iter()
            .flat_map(|other| other.evidence.iter().cloned()),
    );
    finding(
        rule,
        &member.object.id,
        message,
        evidence,
        related
            .iter()
            .filter(|other| other.object.id != member.object.id)
            .map(|other| other.object.id.clone())
            .collect(),
    )
}

fn check_prefix(
    rule: &CompiledRule,
    config: &Config<'_>,
    scope: &Scope<'_>,
    length: usize,
    strays: usize,
    evaluation: &mut CapabilityEvaluation,
) {
    let property = config.property;
    let mut by_prefix: BTreeMap<&str, Vec<&Member<'_>>> = BTreeMap::new();
    for member in &scope.members {
        match member.digits.get(..length) {
            Some(prefix) if member.digits.len() >= length => {
                by_prefix.entry(prefix).or_default().push(member);
            }
            _ => evaluation.push_object_not_evaluated(
                member.object.id.clone(),
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{property} {} has fewer than {length} digit(s), so it has no prefix",
                    member.shown
                ),
            ),
        }
    }
    if by_prefix.len() < 2 {
        return;
    }
    let mut counts: Vec<usize> = by_prefix.values().map(Vec::len).collect();
    counts.sort_unstable_by(|a, b| b.cmp(a));
    let undecided = scope.undecided + strays;
    let predominant = (counts[0] - counts[1] > undecided)
        .then(|| {
            by_prefix
                .iter()
                .find(|(_, members)| members.len() == counts[0])
        })
        .flatten();
    if let Some((prefix, holders)) = predominant {
        for (_, members) in by_prefix.iter().filter(|(other, _)| *other != prefix) {
            for member in members {
                evaluation.push_finding(report(
                        rule,
                        member,
                        format!(
                            "{property} {} does not start with {prefix}, the prefix of {} other object(s)",
                            member.shown,
                            holders.len()
                        ),
                        holders,
                    ));
            }
        }
    } else {
        let prefixes = by_prefix
            .iter()
            .map(|(prefix, members)| format!("{prefix} ({})", members.len()))
            .collect::<Vec<_>>()
            .join(", ");
        let everyone: Vec<&Member<'_>> = by_prefix.values().flatten().copied().collect();
        for member in &everyone {
            evaluation.push_finding(report(
                rule,
                member,
                format!(
                    "{property} {}: its scope mixes the prefixes {prefixes}",
                    member.shown
                ),
                &everyone,
            ));
        }
    }
}

fn check_gaps(
    rule: &CompiledRule,
    config: &Config<'_>,
    scope: &Scope<'_>,
    strays: &[Option<u64>],
    evaluation: &mut CapabilityEvaluation,
) {
    let property = config.property;
    let mut by_number: BTreeMap<u64, Vec<&Member<'_>>> = BTreeMap::new();
    for member in &scope.members {
        by_number.entry(member.number).or_default().push(member);
    }
    let numbers: Vec<(&u64, &Vec<&Member<'_>>)> = by_number.iter().collect();
    for pair in numbers.windows(2) {
        let [(below, under), (above, over)] = pair else {
            continue;
        };
        let (below, above) = (**below, **above);
        if above - below < 2 {
            continue;
        }
        let missing = if above - below == 2 {
            format!("{} is missing", below + 1)
        } else {
            format!("{} to {} are missing", below + 1, above - 1)
        };
        // An unread number might be one of the missing ones.
        let fillable = scope.undecided > 0
            || strays
                .iter()
                .any(|number| number.is_none_or(|number| below < number && number < above));
        for member in *over {
            if fillable {
                evaluation.push_object_not_evaluated(
                    member.object.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{property} {} follows {below}, but an object whose number could not be read may fill the gap",
                        member.shown
                    ),
                );
            } else {
                evaluation.push_finding(report(
                    rule,
                    member,
                    format!("{property} {} follows {below}; {missing}", member.shown),
                    under,
                ));
            }
        }
    }
}
