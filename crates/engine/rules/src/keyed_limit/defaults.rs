//! `keyed-limit`'s `door_type_defaults`: the dimensions a door of each type
//! is taken to have where the model states none.
//!
//! A row applies to a door by its operation type, matched as a text pattern
//! against the operation its leaves state (`ObjectFrameService::leaves`),
//! and by a selector; the first row both match is the door's type row. Its
//! cells are rule parameters, never measurements: each is used only after
//! the door states no value, and every use is named in the outcome's
//! message and cited by an inexact `axioval:default.door-type` evidence
//! entry. A value the door states but that cannot be read is never replaced
//! by a default.

use axioval_engine::{
    ColumnKind, NotEvaluatedReason, ObjectFrameServiceHandle, RuleContext, TableColumn,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, QuantityDimension};

use crate::plan_area::shown;
use crate::selection::{Selection, selector_matches};
use crate::support::table::{Matched, RowSelection, RowTest, TextPattern, match_rows};
use crate::support::{Parameters, Unavailable, invalid, si_quantity};

/// The columns of `door_type_defaults`.
pub(crate) const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("operation", ColumnKind::TextPattern),
    TableColumn::optional("applies_to", ColumnKind::Selector),
    TableColumn::optional("width_deduction", ColumnKind::Quantity),
    TableColumn::optional("height_deduction", ColumnKind::Quantity),
    TableColumn::optional("threshold_height", ColumnKind::Quantity),
    TableColumn::optional("glazing_ratio", ColumnKind::Number),
];

/// One default a type row may give.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Item {
    /// Deducted from the overall width for a clear width.
    WidthDeduction,
    /// Deducted from the overall height in place of a head lining.
    HeightDeduction,
    /// A threshold's height above the door's bottom.
    ThresholdHeight,
    /// The glazed share of the leaf.
    GlazingRatio,
}

impl Item {
    fn column(self) -> &'static str {
        match self {
            Self::WidthDeduction => "width_deduction",
            Self::HeightDeduction => "height_deduction",
            Self::ThresholdHeight => "threshold_height",
            Self::GlazingRatio => "glazing_ratio",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::WidthDeduction => "width deduction",
            Self::HeightDeduction => "height deduction",
            Self::ThresholdHeight => "threshold",
            Self::GlazingRatio => "glazing ratio",
        }
    }
}

/// One row of the defaults table.
struct Row<'a> {
    operation: Option<TextPattern>,
    applies_to: Option<&'a Selector>,
    values: [Option<f64>; 4],
}

impl Row<'_> {
    fn value(&self, which: Item) -> Option<f64> {
        let slot = match which {
            Item::WidthDeduction => 0,
            Item::HeightDeduction => 1,
            Item::ThresholdHeight => 2,
            Item::GlazingRatio => 3,
        };
        self.values[slot]
    }
}

/// The declared defaults per door type.
pub(crate) struct DoorDefaults<'a> {
    rows: Vec<Row<'a>>,
}

/// A default a door's type row gave: the value, how a message words it and
/// the evidence recording its use.
pub(crate) struct Used {
    pub(crate) value: f64,
    pub(crate) words: String,
    pub(crate) evidence: Vec<Evidence>,
}

impl<'a> DoorDefaults<'a> {
    /// The declared table, `None` when `door_type_defaults` is not declared.
    pub(crate) fn parse(
        parameters: &Parameters<'a>,
        case_sensitive: bool,
    ) -> Result<Option<Self>, Unavailable> {
        let Some(table) = parameters.table("door_type_defaults")? else {
            return Ok(None);
        };
        let mut rows = Vec::with_capacity(table.len());
        for (index, row) in table.iter().enumerate() {
            let operation = row.pattern("operation", case_sensitive)?;
            let applies_to = row.selector("applies_to")?;
            let mut values = [None; 4];
            for (slot, which) in [
                Item::WidthDeduction,
                Item::HeightDeduction,
                Item::ThresholdHeight,
            ]
            .into_iter()
            .enumerate()
            {
                let Some((value, unit)) = row.quantity(which.column())? else {
                    continue;
                };
                let (value, dimension) =
                    si_quantity(value, unit).map_err(|(reason, message)| {
                        (reason, format!("door_type_defaults row {index}: {message}"))
                    })?;
                if dimension != QuantityDimension::Length || value < 0.0 {
                    return Err(invalid(format!(
                        "door_type_defaults row {index}: `{}` must be a non-negative length",
                        which.column()
                    )));
                }
                values[slot] = Some(value);
            }
            if let Some(ratio) = row.number("glazing_ratio")? {
                if !(0.0..=1.0).contains(&ratio) {
                    return Err(invalid(format!(
                        "door_type_defaults row {index}: `glazing_ratio` must lie between 0 and 1"
                    )));
                }
                values[3] = Some(ratio);
            }
            rows.push(Row {
                operation,
                applies_to,
                values,
            });
        }
        Ok(Some(Self { rows }))
    }

    /// The default `which` of `door`'s type: `None` when no row applies or
    /// the door's row gives none. A row that may apply before the first
    /// that surely does leaves the door's type, and so the default,
    /// unknown.
    pub(crate) fn lookup(
        &self,
        context: &RuleContext<'_>,
        door: &Object,
        which: Item,
    ) -> Result<Option<Used>, Unavailable> {
        let mut evidence = Vec::new();
        let operation = if self.rows.iter().any(|row| row.operation.is_some()) {
            Some(operation(context, door, &mut evidence))
        } else {
            None
        };
        let mut unknown = Vec::new();
        let matched = match_rows(&self.rows, RowSelection::First, |row| {
            let by_operation = match (&row.operation, &operation) {
                (None, _) => RowTest::Match(0),
                (Some(pattern), Some(Ok(operation))) => pattern.test(operation),
                (Some(_), Some(Err(why))) => {
                    unknown.push(why.clone());
                    RowTest::Undecided
                }
                (Some(_), None) => RowTest::Undecided,
            };
            let by_selector = match row.applies_to {
                None => RowTest::Match(0),
                Some(selector) => match selector_matches(context, selector, door, &mut evidence) {
                    Selection::Match => RowTest::Match(0),
                    Selection::NoMatch => RowTest::NoMatch,
                    Selection::NotEvaluated(_, why) => {
                        unknown.push(format!("its `applies_to` cannot be decided: {why}"));
                        RowTest::Undecided
                    }
                },
            };
            by_operation.and(by_selector)
        });
        let (index, row) = match matched {
            Matched::Rows(rows) => match rows.first() {
                Some(&(index, row)) => (index, row),
                None => return Ok(None),
            },
            Matched::Undecided | Matched::Ambiguous(_) => {
                unknown.sort();
                unknown.dedup();
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "the door states no {} and its type's default cannot be decided: {}",
                        which.noun(),
                        unknown.join("; ")
                    ),
                ));
            }
        };
        let Some(value) = row.value(which) else {
            return Ok(None);
        };
        let mut used = Evidence::exact(
            door.id.source.clone(),
            format!(
                "axioval:default.door-type:{}:row={index};{}={value}",
                door.id,
                which.column()
            ),
        );
        used.exact = false;
        evidence.push(used);
        let shown_value = if which == Item::GlazingRatio {
            shown(value, value)
        } else {
            format!("{} m", shown(value, value))
        };
        Ok(Some(Used {
            value,
            words: format!(
                "the door type's default {} {shown_value} (door_type_defaults row {index})",
                which.noun()
            ),
            evidence,
        }))
    }
}

/// The operation type `door`'s leaves state, or why it is unknown.
fn operation(
    context: &RuleContext<'_>,
    door: &Object,
    evidence: &mut Vec<Evidence>,
) -> Result<String, String> {
    let Some(frames) = context.services.get::<ObjectFrameServiceHandle>() else {
        return Err(
            "the object-frame service is not registered, so the door's operation type is unknown"
                .into(),
        );
    };
    match frames.leaves(&door.id) {
        Ok(leaves) => {
            evidence.push(leaves.evidence().clone());
            Ok(leaves.operation().to_owned())
        }
        Err(error) => Err(format!("the door's operation type is unknown: {error}")),
    }
}
