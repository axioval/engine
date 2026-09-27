//! `recess-width`: each recess of a footprint is wide enough for its depth.
//!
//! A recess is a pocket between an object's footprint and its convex hull,
//! as `PlanSpanService::measure_recesses` finds it: its width is its mouth,
//! its depth how far it reaches behind the mouth. The convex hull is used
//! rather than the minimum-area rectangle because it is orientation-free
//! and every pocket of it is a genuine indentation: a rectangle would also
//! report the corners of a trapezoid or a rounded room as recesses.
//!
//! The applicable requirement is the first row of `requirements` whose depth
//! range holds the recess's depth: deeper than `minimum_depth_metres` and no
//! deeper than `maximum_depth_metres`, either bound optional. The row
//! requires a width of at least `minimum_width_metres`, and at least
//! `minimum_width_per_depth` times the depth; with both, the larger. A
//! recess no row holds has no requirement. A depth interval straddling a
//! row's bound leaves the recess not evaluated, and so does a width interval
//! straddling the requirement.

use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, PlanRecess, PlanSpanError, PlanSpanServiceHandle, RuleCapability, RuleContext,
    TableColumn,
};
use axioval_ir::ObjectId;

use crate::level_spacing::shown;
use crate::selection::select_objects;
use crate::support::table::{Matched, Row, RowSelection, RowTest, match_rows};
use crate::support::{Parameters, Unavailable, finding, invalid};

const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("minimum_depth_metres", ColumnKind::Number),
    TableColumn::optional("maximum_depth_metres", ColumnKind::Number),
    TableColumn::optional("minimum_width_metres", ColumnKind::Number),
    TableColumn::optional("minimum_width_per_depth", ColumnKind::Number),
];

/// Requires every recess of a selected object's footprint to be wide enough
/// for its depth.
pub struct RecessWidth;

/// One row of the requirement table.
struct Requirement {
    deeper_than: Option<f64>,
    at_most: Option<f64>,
    width: f64,
    per_depth: f64,
}

impl Requirement {
    fn read(row: Row<'_>, index: usize) -> Result<Self, Unavailable> {
        let non_negative = |column: &str| -> Result<Option<f64>, Unavailable> {
            match row.number(column)? {
                Some(value) if value < 0.0 => Err(invalid(format!(
                    "row {index}: `{column}` must not be negative"
                ))),
                other => Ok(other),
            }
        };
        let deeper_than = non_negative("minimum_depth_metres")?;
        let at_most = non_negative("maximum_depth_metres")?;
        let width = non_negative("minimum_width_metres")?;
        let per_depth = non_negative("minimum_width_per_depth")?;
        if width.is_none() && per_depth.is_none() {
            return Err(invalid(format!(
                "row {index} needs `minimum_width_metres`, `minimum_width_per_depth` or both"
            )));
        }
        if let (Some(low), Some(high)) = (deeper_than, at_most)
            && low >= high
        {
            return Err(invalid(format!(
                "row {index}: `minimum_depth_metres` must be below `maximum_depth_metres`"
            )));
        }
        Ok(Self {
            deeper_than,
            at_most,
            width: width.unwrap_or(0.0),
            per_depth: per_depth.unwrap_or(0.0),
        })
    }

    /// Whether a depth in `[low, high]` falls in this row's range.
    fn holds(&self, low: f64, high: f64) -> RowTest {
        let above = self.deeper_than.map_or(RowTest::Match(0), |bound| {
            if low > bound {
                RowTest::Match(0)
            } else if high <= bound {
                RowTest::NoMatch
            } else {
                RowTest::Undecided
            }
        });
        let below = self.at_most.map_or(RowTest::Match(0), |bound| {
            if high <= bound {
                RowTest::Match(0)
            } else if low > bound {
                RowTest::NoMatch
            } else {
                RowTest::Undecided
            }
        });
        above.and(below)
    }

    /// The width required for a depth in `[low, high]`, as an interval.
    fn required(&self, low: f64, high: f64) -> (f64, f64) {
        (
            self.width.max(self.per_depth * low),
            self.width.max(self.per_depth * high),
        )
    }
}

fn requirements(rule: &CompiledRule) -> Result<Vec<Requirement>, Unavailable> {
    let rows = Parameters(rule)
        .table("requirements")?
        .ok_or_else(|| invalid("parameter `requirements` is required"))?;
    rows.into_iter()
        .enumerate()
        .map(|(index, row)| Requirement::read(row, index))
        .collect()
}

fn unavailable(object: &ObjectId, error: &PlanSpanError) -> Unavailable {
    let reason = match error {
        PlanSpanError::UnknownObject(_) | PlanSpanError::Unavailable(_) => {
            NotEvaluatedReason::IncompleteEvidence
        }
        PlanSpanError::InvalidMeasurement | PlanSpanError::InexactEvidence => {
            NotEvaluatedReason::InvalidEvidence
        }
    };
    (
        reason,
        format!("the recesses of {object} cannot be measured: {error}"),
    )
}

fn located(recess: &PlanRecess) -> String {
    let [a, b] = recess.mouth();
    format!(
        "recess at ({:.3}, {:.3})-({:.3}, {:.3})",
        a[0], a[1], b[0], b[1]
    )
}

impl RuleCapability for RecessWidth {
    fn id(&self) -> &'static str {
        "axioval:capability.recess-width"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::required(
            "requirements",
            ParameterType::Table(COLUMNS),
        )]
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
