//! `numbering-consistency` as it was implemented before it became a
//! template (#283), kept only as the parity reference the template is held
//! to in the rules crate's tests (`parity-reference` feature). It is no
//! capability of any registry. It reads the numbers through the same
//! `Collected` the template's measured `numbering` reads.

use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, RuleCapability,
    RuleContext,
};

use super::{Collected, Config, Member, NAME, Scope};
use crate::selection::select_objects;
use crate::support::{Parameters, finding};

/// Requires the numbers of one scope to share a prefix and leave no gaps,
/// as `numbering-consistency` judged it before it became a template.
pub struct NumberingConsistency;

impl RuleCapability for NumberingConsistency {
    fn id(&self) -> &'static str {
        super::template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::parameters()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match Config::read(&Parameters(rule)) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(reason, format!("{NAME}: {message}"));
            }
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let collected = Collected::of(context, &config, &selected);
        for (object, reason, message) in &collected.open {
            evaluation.push_object_not_evaluated(object.clone(), reason.clone(), message.clone());
        }
        judge(rule, &config, &collected, &mut evaluation);
        evaluation
    }
}

/// Runs the declared checks on every scope.
fn judge(
    rule: &CompiledRule,
    config: &Config<'_>,
    collected: &Collected<'_>,
    evaluation: &mut CapabilityEvaluation,
) {
    for scope in collected.scopes.values() {
        let strays = collected.strays(scope);
        if let Some(length) = config.prefix_length {
            check_prefix(rule, config, scope, length, strays.len(), evaluation);
        }
        if config.gap_free {
            check_gaps(rule, config, scope, &strays, evaluation);
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
