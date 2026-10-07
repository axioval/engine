//! Special-case tolerances along the elements' own axes.
//!
//! World axes misjudge elements at an angle: a slab edge sunk 10 mm into a
//! wall at 30° reaches far along x and y, so no world-axis tolerance lets it
//! pass. A `tolerance_cases` row names a case, two component filters and a
//! tolerance, and excuses an intersection between a pair matching the
//! filters (`first_selector` on one member, `second_selector` on the other,
//! either way round) whose extent along the elements' own placement axes
//! stays within the tolerance:
//!
//! | Case | Measured along |
//! |---|---|
//! | `horizontal_orthogonal` | the second element's plan axes, the lesser extent |
//! | `vertical_orthogonal` | the second element's vertical axis |
//! | `horizontal_protrusion` | the first element's plan axes, the lesser extent |
//! | `vertical_protrusion` | the first element's vertical axis |
//!
//! An orthogonal case measures how far the first element reaches into the
//! second, across the second's own axes (a slab edge into a wall's
//! thickness); a protrusion how far the first element sticks out, along its
//! own. The axes are the placement frames the object-frame service states,
//! and the extents come from the proximity service
//! ([`axioval_engine::OverlapAlongRequest`]).
//!
//! Everything is three-valued. A case excuses a pair only when its filters
//! surely match and the whole measured extent lies within the tolerance; it
//! leaves the pair open when a filter or the extent is undecided, or when a
//! frame or the extents cannot be read.

use axioval_engine::{ColumnKind, LengthInterval, ParameterDescriptor, ParameterType, TableColumn};
use axioval_ir::Evidence;
use axioval_ir::contract::Selector;

use crate::clash::Holds;
use crate::support::{Parameters, Unavailable, invalid};

const CASE_COLUMNS: &[TableColumn] = &[
    TableColumn::required("case", ColumnKind::String),
    TableColumn::optional("first_selector", ColumnKind::Selector),
    TableColumn::optional("second_selector", ColumnKind::Selector),
    TableColumn::required("tolerance_metres", ColumnKind::Number),
];

/// The `tolerance_cases` parameter `clash` and `clash-matrix` share.
pub(crate) fn case_parameter() -> ParameterDescriptor {
    ParameterDescriptor::optional("tolerance_cases", ParameterType::Table(CASE_COLUMNS))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    HorizontalOrthogonal,
    VerticalOrthogonal,
    HorizontalProtrusion,
    VerticalProtrusion,
}

impl Kind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::HorizontalOrthogonal => "horizontal orthogonal",
            Self::VerticalOrthogonal => "vertical orthogonal",
            Self::HorizontalProtrusion => "horizontal protrusion",
            Self::VerticalProtrusion => "vertical protrusion",
        }
    }
    /// Whether the extent is measured along the second element's axes.
    pub(crate) fn along_second(self) -> bool {
        matches!(self, Self::HorizontalOrthogonal | Self::VerticalOrthogonal)
    }
    pub(crate) fn horizontal(self) -> bool {
        matches!(
            self,
            Self::HorizontalOrthogonal | Self::HorizontalProtrusion
        )
    }
}

pub(crate) struct Case<'a> {
    pub(crate) kind: Kind,
    /// The filters on the first and the second element.
    pub(crate) selectors: [Option<&'a Selector>; 2],
    pub(crate) tolerance: f64,
}

/// The declared tolerance cases.
pub(crate) struct Cases<'a>(pub(crate) Vec<Case<'a>>);

/// Reads `tolerance_cases`; no rows declare no case.
pub(crate) fn cases<'a>(parameters: &Parameters<'a>) -> Result<Cases<'a>, Unavailable> {
    let mut cases = Vec::new();
    for (index, row) in parameters
        .table("tolerance_cases")?
        .unwrap_or_default()
        .into_iter()
        .enumerate()
    {
        let kind = match row.text("case")?.unwrap_or_default() {
            "horizontal_orthogonal" => Kind::HorizontalOrthogonal,
            "vertical_orthogonal" => Kind::VerticalOrthogonal,
            "horizontal_protrusion" => Kind::HorizontalProtrusion,
            "vertical_protrusion" => Kind::VerticalProtrusion,
            other => {
                return Err(invalid(format!(
                    "tolerance case {index}: `{other}` is not `horizontal_orthogonal`, \
                     `vertical_orthogonal`, `horizontal_protrusion` or `vertical_protrusion`"
                )));
            }
        };
        let tolerance = row.number("tolerance_metres")?.unwrap_or(-1.0);
        if !(tolerance >= 0.0 && tolerance.is_finite()) {
            return Err(invalid(format!(
                "tolerance case {index}: `tolerance_metres` must not be negative"
            )));
        }
        cases.push(Case {
            kind,
            selectors: [
                row.selector("first_selector")?,
                row.selector("second_selector")?,
            ],
            tolerance,
        });
    }
    Ok(Cases(cases))
}

impl Cases<'_> {
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Whether a declared case excuses a pair's intersection, why not when it
/// cannot be told, and the evidence read.
pub(crate) struct Excuse {
    pub(crate) holds: Holds,
    /// Why a case may or may not excuse the pair, for an open outcome.
    pub(crate) note: String,
    pub(crate) evidence: Vec<Evidence>,
}

impl Excuse {
    pub(crate) fn none() -> Self {
        Self {
            holds: Holds::No,
            note: String::new(),
            evidence: Vec::new(),
        }
    }
}

/// The lesser of two extents, bound by bound.
pub(crate) fn lesser(a: LengthInterval, b: LengthInterval) -> LengthInterval {
    LengthInterval::try_new(
        a.lower_metres().min(b.lower_metres()),
        a.upper_metres().min(b.upper_metres()),
    )
    .unwrap_or_else(|_| unreachable!("the lesser of two intervals is an interval"))
}

pub(crate) fn described(extent: LengthInterval) -> String {
    if extent.is_exact() {
        format!("{:.4} m", extent.lower_metres())
    } else {
        format!(
            "{:.4} to {:.4} m",
            extent.lower_metres(),
            extent.upper_metres()
        )
    }
}
