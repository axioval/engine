//! `light-well`: spaces stacked into a light well are contiguous, and their
//! shared plan section is large and wide enough for the well's height.
//!
//! Each selected object is a well; its spaces are the objects `member_path`
//! reaches from it (with IFC, the spaces of a zone through
//! `IfcRelAssignsToGroup:forward`). The well is judged on three things:
//!
//! - **contiguity**: ordered by their bottoms, no member starts more than
//!   `gap_tolerance_metres` above the top of the one below it, and the
//!   members share a plan section (the intersection of their footprints,
//!   `PlanSpanService::measure_section`) that is not empty;
//! - **area**: the section's area;
//! - **width**: the short side of the section's least-area rectangle, the
//!   one `measure_rectangle` answers for a footprint. A section whose
//!   rectangle is tied has no known width, and the service refuses it: the
//!   well is then not evaluated.
//!
//! The well's height runs from its lowest bottom to its highest top. The
//! applicable row of `requirements` is the first whose
//! `maximum_height_metres` the height does not exceed (a row without one
//! holds any height); it requires `minimum_area_square_metres` and
//! `minimum_width_metres`. No row means no requirement. Every value is an
//! interval, and one straddling a bound decides nothing.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    PlanSpanError, RuleCapability, RuleContext, TableColumn,
};
use axioval_ir::contract::{ParameterValue, TableRow};

#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

use crate::support::table::{Row, RowTest};
use crate::support::{Unavailable, invalid};

const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("maximum_height_metres", ColumnKind::Number),
    TableColumn::optional("minimum_area_square_metres", ColumnKind::Number),
    TableColumn::optional("minimum_width_metres", ColumnKind::Number),
];

/// Requires stacked light-well spaces to be contiguous and large enough.
///
/// It runs as a template ([`axioval_engine::template`]): the gaps between
/// the measured stack's consecutive spaces against the tolerance, then the
/// measured section, empty or judged by its area and width against the row
/// its height selects.
pub struct LightWell;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for LightWell {
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
    up_to: Option<f64>,
    pub(crate) area: Option<f64>,
    pub(crate) width: Option<f64>,
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
        let requirement = Self {
            up_to: non_negative("maximum_height_metres")?,
            area: non_negative("minimum_area_square_metres")?,
            width: non_negative("minimum_width_metres")?,
        };
        if requirement.area.is_none() && requirement.width.is_none() {
            return Err(invalid(format!(
                "row {index} needs `minimum_area_square_metres`, `minimum_width_metres` or both"
            )));
        }
        Ok(requirement)
    }

    /// Whether a well `[low, high]` high falls in this row's range.
    pub(crate) fn holds(&self, low: f64, high: f64) -> RowTest {
        match self.up_to {
            None => RowTest::Match(0),
            Some(bound) if high <= bound => RowTest::Match(0),
            Some(bound) if low > bound => RowTest::NoMatch,
            Some(_) => RowTest::Undecided,
        }
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

/// Checks the rows the measured well is handed (`requirements`), as the
/// rule states them: a row refused in the capability's words.
pub(crate) fn check_arguments(
    arguments: &std::collections::BTreeMap<String, ParameterValue>,
) -> Result<(), Unavailable> {
    match arguments.get("requirements") {
        Some(ParameterValue::Table { value }) => rows(value).map(|_| ()),
        Some(_) => Err(invalid("table column `requirements` has the wrong type")),
        None => Ok(()),
    }
}

/// Why the plan section the stacked spaces share cannot be measured.
pub(crate) fn section_unavailable(error: &PlanSpanError) -> Unavailable {
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
        format!("the shared plan section cannot be measured: {error}"),
    )
}
