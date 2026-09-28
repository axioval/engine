//! The dimensioning table: rows bounding the distance from an opening to
//! the nearest other opening of its host, or to one of the host's edges,
//! along one of the face's axes.

use axioval_engine::{
    CapabilityEvaluation, ColumnKind, Deviation, NotEvaluatedReason, RuleContext, TableColumn,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Object, ObjectId, QuantityDimension};

use super::face::{FaceAxes, Host, ROUNDING, Span, gap};
use super::{Judge, Placed, list};
use crate::counts::Population;
use crate::level_spacing::metres;
use crate::support::table::Row;
use crate::support::{Parameters, Unavailable, invalid, si_quantity};

/// The columns of `dimensions`.
pub(super) const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("name", ColumnKind::String),
    TableColumn::required("source", ColumnKind::Selector),
    TableColumn::optional("target", ColumnKind::Selector),
    TableColumn::optional("edge", ColumnKind::String),
    TableColumn::optional("direction", ColumnKind::String),
    TableColumn::optional("minimum", ColumnKind::Quantity),
    TableColumn::optional("maximum", ColumnKind::Quantity),
    TableColumn::optional("fixed", ColumnKind::Quantity),
    TableColumn::optional("tolerance", ColumnKind::Quantity),
    TableColumn::optional("overlap", ColumnKind::Boolean),
];

/// A face axis a distance runs along.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Along {
    Length,
    Height,
}

impl Along {
    fn name(self) -> &'static str {
        match self {
            Self::Length => "length",
            Self::Height => "height",
        }
    }
}

/// One of the host's edges.
#[derive(Clone, Copy)]
enum Edge {
    /// The high end of the height axis.
    Top,
    /// The low end of the height axis.
    Bottom,
    /// The nearer end along the length axis.
    Side,
}

impl Edge {
    fn name(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Side => "side",
        }
    }
}

/// What a row measures to.
enum To<'a> {
    Openings {
        selector: &'a Selector,
        /// Only openings overlapping the measured one across the direction.
        overlap: bool,
    },
    Edge(Edge),
}

/// The bound a row puts on a distance.
#[derive(Clone, Copy)]
enum Bound {
    Range {
        minimum: Option<f64>,
        maximum: Option<f64>,
    },
    Fixed(f64),
}

/// One row of `dimensions`.
pub(super) struct Dimension<'a> {
    label: String,
    source: &'a Selector,
    to: To<'a>,
    along: Along,
    bound: Bound,
    tolerance: f64,
}

fn length_cell(row: Row<'_>, column: &str, number: usize) -> Result<Option<f64>, Unavailable> {
    match row.quantity(column)? {
        None => Ok(None),
        Some((value, unit)) => match si_quantity(value, unit)? {
            (value, QuantityDimension::Length) if value >= 0.0 => Ok(Some(value)),
            _ => Err(invalid(format!(
                "`dimensions` row {number}'s `{column}` is not a non-negative length"
            ))),
        },
    }
}

/// Reads `dimensions`; none when it is absent.
pub(super) fn parse<'a>(parameters: &Parameters<'a>) -> Result<Vec<Dimension<'a>>, Unavailable> {
    let Some(rows) = parameters.table("dimensions")? else {
        return Ok(Vec::new());
    };
    rows.into_iter()
        .enumerate()
        .map(|(index, row)| parse_row(row, index + 1))
        .collect()
}

fn parse_row(row: Row<'_>, number: usize) -> Result<Dimension<'_>, Unavailable> {
    let row_invalid = |message: &str| invalid(format!("`dimensions` row {number}: {message}"));
    let source = row
        .selector("source")?
        .ok_or_else(|| row_invalid("`source` is required"))?;
    let direction = match row.text("direction")? {
        None => None,
        Some("length") => Some(Along::Length),
        Some("height") => Some(Along::Height),
        Some(other) => {
            return Err(row_invalid(&format!(
                "`direction` `{other}` is unsupported; use `length` or `height`"
            )));
        }
    };
    let overlap = row.boolean("overlap")?;
    let (to, along) = match (row.selector("target")?, row.text("edge")?) {
        (Some(selector), None) => (
            To::Openings {
                selector,
                overlap: overlap.unwrap_or(false),
            },
            direction.ok_or_else(|| {
                row_invalid("a `target` needs a `direction`, `length` or `height`")
            })?,
        ),
        (None, Some(edge)) => {
            if overlap.is_some() {
                return Err(row_invalid(
                    "`overlap` applies to a `target`, not an `edge`",
                ));
            }
            let (edge, along) = match edge {
                "top" => (Edge::Top, Along::Height),
                "bottom" => (Edge::Bottom, Along::Height),
                "side" => (Edge::Side, Along::Length),
                other => {
                    return Err(row_invalid(&format!(
                        "`edge` `{other}` is unsupported; use `top`, `bottom` or `side`"
                    )));
                }
            };
            if direction.is_some_and(|direction| direction != along) {
                return Err(row_invalid(&format!(
                    "the `{}` edge is measured along the `{}`",
                    edge.name(),
                    along.name()
                )));
            }
            (To::Edge(edge), along)
        }
        _ => return Err(row_invalid("state exactly one of `target` and `edge`")),
    };
    let bound = match (
        length_cell(row, "minimum", number)?,
        length_cell(row, "maximum", number)?,
        length_cell(row, "fixed", number)?,
    ) {
        (None, None, None) => {
            return Err(row_invalid(
                "state a `minimum`, a `maximum` or a `fixed` distance",
            ));
        }
        (None, None, Some(fixed)) => Bound::Fixed(fixed),
        (_, _, Some(_)) => {
            return Err(row_invalid(
                "a `fixed` distance excludes `minimum` and `maximum`",
            ));
        }
        (Some(minimum), Some(maximum), None) if minimum > maximum => {
            return Err(row_invalid("`minimum` exceeds `maximum`"));
        }
        (minimum, maximum, None) => Bound::Range { minimum, maximum },
    };
    Ok(Dimension {
        label: match row.text("name")? {
            Some(name) if !name.trim().is_empty() => format!("dimension `{}`", name.trim()),
            _ => format!("dimension row {number}"),
        },
        source,
        to,
        along,
        bound,
        tolerance: length_cell(row, "tolerance", number)?.unwrap_or(0.0),
    })
}

/// The selections a table needs, per row: the openings it measures from,
/// and those it measures to.
pub(super) struct Selections(Vec<(Population, Option<Population>)>);

impl Selections {
    pub(super) fn of(context: &RuleContext<'_>, dimensions: &[Dimension<'_>]) -> Self {
        Self(
            dimensions
                .iter()
                .map(|dimension| {
                    (
                        Population::of(context, dimension.source),
                        match dimension.to {
                            To::Openings { selector, .. } => {
                                Some(Population::of(context, selector))
                            }
                            To::Edge(_) => None,
                        },
                    )
                })
                .collect(),
        )
    }
}

/// How a distance meets a row's bound.
enum Verdict {
    Holds,
    /// A finding: what is required, and how far it misses.
    Misses(String, Deviation),
    Undecided,
}

impl Bound {
    /// Judges a distance known to lie in `[lower, upper]`: it holds or
    /// misses only when every value there does. The tolerance widens the
    /// bound; the deviation is measured from the bound as declared.
    fn judge(self, (lower, upper): Span, tolerance: f64) -> Verdict {
        let slack = tolerance + ROUNDING;
        let (minimum, maximum) = match self {
            Self::Range { minimum, maximum } => (minimum, maximum),
            Self::Fixed(fixed) => (Some(fixed), Some(fixed)),
        };
        let required = match self {
            Self::Fixed(fixed) => format!("{} required", metres(fixed)),
            Self::Range {
                minimum: Some(minimum),
                maximum: Some(maximum),
            } => format!("{} to {} allowed", metres(minimum), metres(maximum)),
            Self::Range {
                minimum: Some(minimum),
                maximum: None,
            } => format!("at least {} required", metres(minimum)),
            Self::Range { maximum, .. } => {
                format!("at most {} allowed", metres(maximum.unwrap_or(0.0)))
            }
        };
        let required = if tolerance > 0.0 {
            format!("{required} within {}", metres(tolerance))
        } else {
            required
        };
        if let Some(minimum) = minimum
            && upper < minimum - slack
        {
            return Verdict::Misses(required, Deviation::below(minimum, lower, upper));
        }
        if let Some(maximum) = maximum
            && lower > maximum + slack
        {
            return Verdict::Misses(required, Deviation::above(maximum, lower, upper));
        }
        let above_minimum = minimum.is_none_or(|minimum| lower >= minimum - slack);
        let below_maximum = maximum.is_none_or(|maximum| upper <= maximum + slack);
        if above_minimum && below_maximum {
            Verdict::Holds
        } else {
            Verdict::Undecided
        }
    }
}

/// How far an opening is from its host's `edge`, as an interval: exact,
/// or from below where the host's free outline is known only within the
/// box the opening may lie in. `None` where the outline passes through the
/// opening, which is found apart.
fn edge_distance(
    host: &Host,
    placed: &Placed,
    rect: [Span; 2],
    edge: Edge,
    axes: FaceAxes,
) -> Option<Span> {
    let (measured, extent) = match edge {
        Edge::Top | Edge::Bottom => (axes.height, placed.height),
        Edge::Side => (axes.length, placed.length),
    };
    let (low, high) = host.clearance(measured, extent, rect)?;
    let value = match edge {
        Edge::Top => high,
        Edge::Bottom => low,
        Edge::Side => low.min(high),
    };
    let exact = placed.section_exact || !host.outlined(measured);
    Some((value, if exact { value } else { f64::INFINITY }))
}

/// A row's finding: its message, deviation, and the opening it names.
type Missed<'p> = (String, Deviation, Option<(&'p ObjectId, &'p Placed)>);

impl Judge<'_, '_> {
    /// Judges every row of the dimensioning table whose `source` selects
    /// the opening.
    pub(super) fn dimensions(
        &self,
        opening: &Object,
        placed: &Placed,
        host: &Host,
        rect: [Span; 2],
        evaluation: &mut CapabilityEvaluation,
    ) {
        for (dimension, (sources, targets)) in self.config.dimensions.iter().zip(&self.dimensions.0)
        {
            if sources.undecided.contains(&opening.id) {
                evaluation.push_object_not_evaluated(
                    opening.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("whether {} applies to it is undecided", dimension.label),
                );
                continue;
            }
            if !sources.matched.contains(&opening.id) {
                continue;
            }
            let judged = match (&dimension.to, targets) {
                (To::Edge(edge), _) => {
                    Self::edge_row(placed, host, rect, dimension, *edge, self.config.axes)
                }
                (To::Openings { overlap, .. }, Some(targets)) => {
                    self.target_row(opening, placed, dimension, *overlap, targets)
                }
                (To::Openings { .. }, None) => Ok(None),
            };
            match judged {
                Ok(None) => {}
                Ok(Some((message, deviation, target))) => {
                    let mut evidence = placed.evidence.clone();
                    let mut related = Vec::new();
                    if let Some((target, neighbour)) = target {
                        evidence.extend(neighbour.evidence.iter().cloned());
                        related.push(target.clone());
                    }
                    evaluation.push_graded_finding(
                        self.finding(opening, placed, message, &evidence, &related),
                        deviation,
                    );
                }
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(opening.id.clone(), reason, message);
                }
            }
        }
    }

    fn edge_row<'p>(
        placed: &Placed,
        host: &Host,
        rect: [Span; 2],
        dimension: &Dimension<'_>,
        edge: Edge,
        axes: FaceAxes,
    ) -> Result<Option<Missed<'p>>, Unavailable> {
        let Some(distance) = edge_distance(host, placed, rect, edge, axes) else {
            return Ok(None);
        };
        let exact = distance.1.is_finite();
        match dimension.bound.judge(distance, dimension.tolerance) {
            Verdict::Holds => Ok(None),
            Verdict::Misses(required, deviation) => Ok(Some((
                format!(
                    "opening is {}{} from the {} of its host {} along its {}; {required} ({})",
                    if exact { "" } else { "at least " },
                    metres(distance.0.max(0.0)),
                    edge.name(),
                    placed.host.local_id,
                    dimension.along.name(),
                    dimension.label
                ),
                deviation,
                None,
            ))),
            Verdict::Undecided => Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "its distance from the {} of its host {} may miss {}: where it lies in the \
                     host's section is known only within bounds",
                    edge.name(),
                    placed.host.local_id,
                    dimension.label
                ),
            )),
        }
    }

    /// Judges a row measured to the nearest target opening of the host.
    /// The nearest sure target bounds that distance from above, and every
    /// undecided one below it; an opening whose place is unknown may be
    /// nearer still. A host with no target leaves the row unjudged.
    fn target_row<'p>(
        &'p self,
        opening: &Object,
        placed: &Placed,
        dimension: &Dimension<'_>,
        overlap: bool,
        targets: &Population,
    ) -> Result<Option<Missed<'p>>, Unavailable> {
        let (own, own_across) = split(placed, dimension.along);
        let mut sure: Vec<(f64, &ObjectId, &Placed)> = Vec::new();
        let mut possible: Vec<(f64, &ObjectId)> = Vec::new();
        let mut unknown: Vec<ObjectId> = Vec::new();
        for (other, neighbour) in self.neighbours(opening, placed) {
            if !targets.contains(other) {
                continue;
            }
            let Some(neighbour) = neighbour else {
                unknown.push(other.clone());
                continue;
            };
            let (theirs, their_across) = split(neighbour, dimension.along);
            if overlap && overlap_of(own_across, their_across) <= ROUNDING {
                continue;
            }
            let distance = gap(own, theirs);
            if targets.matched.contains(other) {
                sure.push((distance, other, neighbour));
            } else {
                possible.push((distance, other));
            }
        }
        sure.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(b.1)));
        let Some(&(nearest, target, neighbour)) = sure.first() else {
            if possible.is_empty() && unknown.is_empty() {
                return Ok(None);
            }
            unknown.extend(possible.into_iter().map(|(_, other)| other.clone()));
            return Err(undecided(dimension, &unknown));
        };
        let lower = if unknown.is_empty() {
            possible
                .iter()
                .map(|(distance, _)| *distance)
                .fold(nearest, f64::min)
        } else {
            0.0
        };
        unknown.extend(
            possible
                .into_iter()
                .filter(|(distance, _)| *distance < nearest)
                .map(|(_, other)| other.clone()),
        );
        match dimension.bound.judge((lower, nearest), dimension.tolerance) {
            Verdict::Holds => Ok(None),
            Verdict::Misses(required, deviation) => Ok(Some((
                format!(
                    "opening is {} from opening {} along the {} of its host {}; {required} ({})",
                    metres(nearest),
                    target.local_id,
                    dimension.along.name(),
                    placed.host.local_id,
                    dimension.label
                ),
                deviation,
                Some((target, neighbour)),
            ))),
            Verdict::Undecided => Err(undecided(dimension, &unknown)),
        }
    }
}

/// An opening's extent along the row's direction and across it.
fn split(placed: &Placed, along: Along) -> (Span, Span) {
    match along {
        Along::Length => (placed.length, placed.height),
        Along::Height => (placed.height, placed.length),
    }
}

fn overlap_of(a: Span, b: Span) -> f64 {
    a.1.min(b.1) - a.0.max(b.0)
}

fn undecided(dimension: &Dimension<'_>, unknown: &[ObjectId]) -> Unavailable {
    (
        NotEvaluatedReason::IncompleteEvidence,
        format!(
            "{} may be measured to {}, whose place or selection is undecided",
            dimension.label,
            list(unknown)
        ),
    )
}
