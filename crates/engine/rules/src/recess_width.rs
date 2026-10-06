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

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    PlanRecess, PlanSpanError, RuleCapability, RuleContext, TableColumn,
};
use axioval_ir::ObjectId;
use axioval_ir::contract::{ParameterValue, TableRow};

#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

use crate::support::table::{Row, RowTest};
use crate::support::{Unavailable, invalid};

const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("minimum_depth_metres", ColumnKind::Number),
    TableColumn::optional("maximum_depth_metres", ColumnKind::Number),
    TableColumn::optional("minimum_width_metres", ColumnKind::Number),
    TableColumn::optional("minimum_width_per_depth", ColumnKind::Number),
];

/// Requires every recess of a selected object's footprint to be wide enough
/// for its depth.
///
/// It runs as a template ([`axioval_engine::template`]): the items of the
/// measured `recesses` list, each with the row its depth selects and the
/// width that row requires, judged by their width.
pub struct RecessWidth;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for RecessWidth {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        TEMPLATE.parameters.clone()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        crate::templates::run((&TEMPLATE, &PLANS), context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&TEMPLATE)
    }
}

/// One row of the requirement table.
pub(crate) struct Requirement {
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
    pub(crate) fn holds(&self, low: f64, high: f64) -> RowTest {
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
    pub(crate) fn required(&self, low: f64, high: f64) -> (f64, f64) {
        (
            self.width.max(self.per_depth * low),
            self.width.max(self.per_depth * high),
        )
    }
}

/// The rows of a requirement table, each read and refused as the
/// capability always read them.
pub(crate) fn rows(table: &[TableRow]) -> Result<Vec<Requirement>, Unavailable> {
    table
        .iter()
        .enumerate()
        .map(|(index, row)| Requirement::read(Row(row), index))
        .collect()
}

/// Checks the rows the measured `recesses` are handed (`requirements`), as
/// the rule states them: a row refused in the capability's words.
pub(crate) fn check_arguments(
    arguments: &std::collections::BTreeMap<String, ParameterValue>,
) -> Result<(), Unavailable> {
    match arguments.get("requirements") {
        Some(ParameterValue::Table { value }) => rows(value).map(|_| ()),
        Some(_) => Err(invalid("table column `requirements` has the wrong type")),
        None => Ok(()),
    }
}

/// Why the recesses of `object` cannot be measured.
pub(crate) fn unavailable(object: &ObjectId, error: &PlanSpanError) -> Unavailable {
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

/// Where a recess's mouth lies, as findings name it.
pub(crate) fn located(recess: &PlanRecess) -> String {
    let [a, b] = recess.mouth();
    format!(
        "recess at ({:.3}, {:.3})-({:.3}, {:.3})",
        a[0], a[1], b[0], b[1]
    )
}
