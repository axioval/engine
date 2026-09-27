//! The trusted outcome refiner: what a rule instance declares about its
//! outcomes that needs selectors, properties or relationships to apply.
//!
//! The runtime grades severities by deviation itself; everything here reads
//! the model, so it lives beside the selectors it evaluates. Every step fails
//! closed: what it cannot decide turns the outcome into a not-evaluated one,
//! never a default.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, OutcomeRefiner, RuleContext,
    RuleRefinement, report_severity,
};
use axioval_ir::contract::{CategoryLevel, SeverityOverride};
use axioval_ir::{Evidence, Finding, Object, Scope, Severity};

use crate::selection::{Selection, selector_matches};
use crate::support::category_headings;

/// Applies a rule instance's severity overrides, then its nested
/// categories, to every finding.
///
/// Registered by [`crate::register_builtins`]; a host registering
/// capabilities one by one registers it with
/// [`axioval_engine::CapabilityRegistry::with_refiner`], or rules declaring
/// refinements fail compilation.
pub struct Refiner;

impl OutcomeRefiner for Refiner {
    fn refine(
        &self,
        context: &RuleContext<'_>,
        _rule: &CompiledRule,
        refinement: &RuleRefinement,
        evaluation: &mut CapabilityEvaluation,
    ) {
        let (overrides, categories) = (&refinement.severity_overrides, &refinement.categories);
        if overrides.is_empty() && categories.is_empty() {
            return;
        }
        for finding in evaluation.take_findings() {
            let refined = if overrides.is_empty() {
                Ok(finding)
            } else {
                overridden(context, overrides, finding)
            };
            let refined = match refined {
                Ok(finding) if !categories.is_empty() => categorised(context, categories, finding),
                refined => refined,
            };
            match refined {
                Ok(finding) => evaluation.push_finding(finding),
                Err(Undecided {
                    scope,
                    reason,
                    message,
                }) => evaluation.push_not_evaluated_about(scope, reason, message),
            }
        }
    }
}

/// `finding` with its subject's nested category headings before its
/// message. A finding about a source or the project has no subject to
/// categorise and is kept as it is.
fn categorised(
    context: &RuleContext<'_>,
    levels: &[CategoryLevel],
    mut finding: Finding,
) -> Result<Finding, Undecided> {
    let Some(subject) = finding
        .object_id()
        .and_then(|id| context.project.object(id))
    else {
        return Ok(finding);
    };
    match category_headings(context, subject, levels) {
        Ok((headings, mut cited)) => {
            finding.message.insert_str(0, &headings);
            cited.extend(std::mem::take(&mut finding.evidence));
            Ok(finding.with_evidence(cited))
        }
        Err((reason, why)) => Err(Undecided {
            scope: finding.scope.clone(),
            reason,
            message: format!(
                "the finding's category cannot be read: {why}; finding: {}",
                finding.message
            ),
        }),
    }
}

/// A finding whose severity could not be decided.
struct Undecided {
    scope: Scope,
    reason: NotEvaluatedReason,
    message: String,
}

/// `finding` with the severity of the first override whose selector holds
/// for its subject or any related object.
///
/// Overrides are tried in order. One holding decides; one that cannot be
/// decided for some involved object may or may not hold, so its severity
/// and every later possibility remain. The finding keeps a severity only
/// when every possibility agrees; otherwise it is not evaluated.
fn overridden(
    context: &RuleContext<'_>,
    overrides: &[SeverityOverride],
    mut finding: Finding,
) -> Result<Finding, Undecided> {
    let involved: Vec<&Object> = finding
        .object_id()
        .into_iter()
        .chain(&finding.related)
        .filter_map(|id| context.project.object(id))
        .collect();
    let mut possible: Vec<Severity> = Vec::new();
    let mut doubt: Option<(NotEvaluatedReason, String)> = None;
    let mut cited: Vec<Evidence> = Vec::new();
    let mut decided = false;
    for (index, rule) in overrides.iter().enumerate() {
        let severity = report_severity(&rule.severity);
        let mut undecided = None;
        let mut evidence = Vec::new();
        for object in &involved {
            match selector_matches(context, &rule.selector, object, &mut evidence) {
                Selection::Match => {
                    decided = true;
                    break;
                }
                Selection::NoMatch => {}
                Selection::NotEvaluated(reason, message) => {
                    undecided.get_or_insert((
                        reason,
                        format!(
                            "severity override {index} cannot be decided for {}: {message}",
                            object.id
                        ),
                    ));
                }
            }
        }
        if decided {
            possible.push(severity);
            cited = evidence;
            break;
        }
        if let Some(why) = undecided {
            possible.push(severity);
            doubt.get_or_insert(why);
        }
    }
    if !decided {
        possible.push(finding.severity.clone());
    }
    possible.sort();
    possible.dedup();
    match (possible.as_slice(), doubt) {
        ([only], _) => {
            if finding.severity != *only {
                finding.severity = only.clone();
                cited.extend(std::mem::take(&mut finding.evidence));
                finding = finding.with_evidence(cited);
            }
            Ok(finding)
        }
        (_, Some((reason, why))) => Err(Undecided {
            scope: finding.scope.clone(),
            reason,
            message: format!(
                "the severity is {} as far as can be decided: {why}; finding: {}",
                possible.iter().map(label).collect::<Vec<_>>().join(" or "),
                finding.message
            ),
        }),
        (_, None) => unreachable!("several severities remain only with an undecided override"),
    }
}

fn label(severity: &Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}
