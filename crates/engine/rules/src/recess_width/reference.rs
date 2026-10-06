//! `recess-width` as it was implemented before it became a template
//! (#283), kept only as the parity reference the template is held to in the
//! rules crate's tests (`parity-reference` feature). It is no capability of
//! any registry. Its rows are read and selected as the template's measured
//! recesses read and select them.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    PlanSpanServiceHandle, RuleCapability, RuleContext,
};

use super::{Requirement, located, unavailable};
use crate::level_spacing::shown;
use crate::selection::select_objects;
use crate::support::table::{Matched, RowSelection, match_rows};
use crate::support::{Parameters, Unavailable, finding, invalid};

fn requirements(rule: &CompiledRule) -> Result<Vec<Requirement>, Unavailable> {
    let rows = Parameters(rule)
        .table("requirements")?
        .ok_or_else(|| invalid("parameter `requirements` is required"))?;
    rows.into_iter()
        .enumerate()
        .map(|(index, row)| Requirement::read(row, index))
        .collect()
}

/// Requires every recess of a selected object's footprint to be wide enough
/// for its depth, as `recess-width` judged it before it became a template.
pub struct RecessWidth;

impl RuleCapability for RecessWidth {
    fn id(&self) -> &'static str {
        super::template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::template::parameters()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let rows = match requirements(rule) {
            Ok(rows) => rows,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("recess-width: {message}"),
                );
            }
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let Some(spans) = context.services.get::<PlanSpanServiceHandle>() else {
            for object in selected {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::MissingService,
                    "plan-span service is not registered",
                );
            }
            return evaluation;
        };
        for object in selected {
            let found = match spans.measure_recesses(&object.id) {
                Ok(found) => found,
                Err(error) => {
                    let (reason, message) = unavailable(&object.id, &error);
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    continue;
                }
            };
            for recess in found.recesses() {
                let (depth_low, depth_high) =
                    (recess.depth().lower_metres(), recess.depth().upper_metres());
                let (width_low, width_high) =
                    (recess.width().lower_metres(), recess.width().upper_metres());
                let described = format!(
                    "{} is {} wide and {} deep",
                    located(recess),
                    shown(width_low, width_high),
                    shown(depth_low, depth_high)
                );
                let (index, row) = match match_rows(&rows, RowSelection::First, |row| {
                    row.holds(depth_low, depth_high)
                }) {
                    Matched::Rows(rows) => match rows.first() {
                        Some((index, row)) => (*index, *row),
                        None => continue,
                    },
                    Matched::Undecided | Matched::Ambiguous(_) => {
                        evaluation.push_object_not_evaluated(
                            object.id.clone(),
                            NotEvaluatedReason::IncompleteEvidence,
                            format!("{described}; which row applies is undecided"),
                        );
                        continue;
                    }
                };
                let (needed_low, needed_high) = row.required(depth_low, depth_high);
                if width_low >= needed_high {
                    continue;
                }
                let needed = shown(needed_low, needed_high);
                if width_high < needed_low {
                    let mut evidence = vec![
                        found.evidence().clone(),
                        recess.width().evidence().clone(),
                        recess.depth().evidence().clone(),
                    ];
                    evidence.dedup();
                    evaluation.push_finding(finding(
                        rule,
                        &object.id,
                        format!("{described}; row {index} requires at least {needed}"),
                        evidence,
                        Vec::new(),
                    ));
                } else {
                    evaluation.push_object_not_evaluated(
                        object.id.clone(),
                        NotEvaluatedReason::IncompleteEvidence,
                        format!("{described}; row {index} requires at least {needed}, undecided"),
                    );
                }
            }
        }
        evaluation
    }
}
