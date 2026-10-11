//! Reading a table parameter at keys that may fall between its rows.
//!
//! A table is rows of cells by column ID. Each [`Axis`] names a key column
//! and the key it is read at, matched exactly or interpolated linearly
//! between the rows whose keys bracket it. Every row states every axis's
//! key, and the cells read form a grid: one row per combination of keys,
//! the grid lines of an interpolated axis being all its rows' keys.
//!
//! Interpolation is linear in each key, so between two grid lines the
//! value is monotone along every axis. Over a key interval (or a box of
//! them) the value therefore takes its extremes at the interval's ends and
//! at the grid lines within, and the result is the hull of the values
//! there. Each value is computed with outward rounding, so the hull holds
//! every value the keys allow.

use axioval_ir::contract::{ScalarValue, TableRow};

use super::evaluate::{Reason, Value, interval_reason};
use super::interval::Interval;
use super::unit::Unit;

/// One key a table is read at.
pub(crate) struct Axis<'a> {
    /// The key column, by ID.
    pub(crate) column: &'a str,
    /// The key.
    pub(crate) key: &'a Value,
    /// Whether the key is interpolated between the rows bracketing it,
    /// rather than matched exactly.
    pub(crate) interpolate: bool,
}

/// The `column` cell of `rows` (the table parameter `table`) at `axes`.
///
/// # Errors
///
/// [`Reason::Unreadable`] for a key beyond the rows' keys without `clamp`,
/// a grid point with no row or no `column` cell, rows stating the same keys,
/// a row stating no key, or an exact key that may match several rows;
/// [`Reason::Mismatch`] for a key of another unit or kind than its column,
/// or a `column` that interpolation meets holding no number.
pub(crate) fn read(
    rows: &[TableRow],
    table: &str,
    axes: &[Axis<'_>],
    column: &str,
    clamp: bool,
) -> Result<Value, Reason> {
    let mut keys = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let mut row_keys = Vec::with_capacity(axes.len());
        for axis in axes {
            row_keys.push(cell(row, axis.column).ok_or_else(|| {
                Reason::Unreadable(format!(
                    "row {} of table `{table}` states no `{}`",
                    index + 1,
                    axis.column
                ))
            })?);
        }
        keys.push(row_keys);
    }
    let mut grid = Grid {
        rows,
        keys,
        lines: Vec::with_capacity(axes.len()),
        table,
        column,
    };
    let mut points = Vec::with_capacity(axes.len());
    for (index, axis) in axes.iter().enumerate() {
        let (at, lines) = grid.points(index, axis, clamp)?;
        points.push(at);
        grid.lines.push(lines);
    }
    // Every combination of one point per axis, the last axis fastest.
    let mut values = Vec::new();
    let mut at = vec![0; axes.len()];
    loop {
        let combination: Vec<&Value> = at
            .iter()
            .zip(&points)
            .map(|(&index, points)| &points[index])
            .collect();
        let candidates: Vec<usize> = (0..rows.len()).collect();
        values.push(grid.at(axes, &combination, &candidates, &mut Vec::new())?);
        let Some(turning) = (0..axes.len())
            .rev()
            .find(|&index| at[index] + 1 < points[index].len())
        else {
            break;
        };
        at[turning] += 1;
        at[turning + 1..].iter_mut().for_each(|index| *index = 0);
    }
    hull(values, column)
}

/// The rows of a table with each one's keys, one per axis.
struct Grid<'a> {
    rows: &'a [TableRow],
    keys: Vec<Vec<Value>>,
    /// Each interpolated axis's distinct row keys, ascending: its grid
    /// lines. Empty for an axis matched exactly.
    lines: Vec<Vec<f64>>,
    table: &'a str,
    column: &'a str,
}

impl Grid<'_> {
    /// The keys `axis` is read at, and its grid lines: the key itself
    /// when matched exactly; else the ends of the key interval (clamped to
    /// the grid lines when `clamp`) and every grid line within.
    fn points(
        &self,
        index: usize,
        axis: &Axis<'_>,
        clamp: bool,
    ) -> Result<(Vec<Value>, Vec<f64>), Reason> {
        let column = axis.column;
        if !axis.interpolate {
            if let Value::Number { value, unit } = axis.key
                && !value.is_point()
                && self.keys.iter().any(|keys| {
                    matches!(&keys[index], Value::Number { value: cell, unit: own }
                        if own == unit && value.contains(cell.lower))
                })
            {
                return Err(Reason::Unreadable(format!(
                    "`{column}` {} cannot be matched to one row's key",
                    axis.key
                )));
            }
            return Ok((vec![axis.key.clone()], Vec::new()));
        }
        let Value::Number { value, unit } = axis.key else {
            return Err(Reason::Mismatch(format!(
                "`{column}` is interpolated between numbers, not {}",
                axis.key.kind()
            )));
        };
        let mut lines = Vec::with_capacity(self.keys.len());
        for (row, keys) in self.keys.iter().enumerate() {
            match &keys[index] {
                Value::Number {
                    value: cell,
                    unit: own,
                } if own == unit && cell.is_point() => lines.push(cell.lower),
                Value::Number { unit: own, .. } if own != unit => {
                    return Err(Reason::Mismatch(format!(
                        "`{column}` is read at {unit}, but row {} of table `{}` states {own}",
                        row + 1,
                        self.table
                    )));
                }
                other => {
                    return Err(Reason::Mismatch(format!(
                        "row {} of table `{}` states `{column}` as {}, not one number",
                        row + 1,
                        self.table,
                        other.kind()
                    )));
                }
            }
        }
        lines.sort_by(f64::total_cmp);
        lines.dedup();
        let (Some(&first), Some(&last)) = (lines.first(), lines.last()) else {
            return Err(Reason::Unreadable(format!(
                "table `{}` has no rows",
                self.table
            )));
        };
        let shown = |at: f64| Value::Number {
            value: Interval::point(at),
            unit: unit.clone(),
        };
        if !clamp && (value.lower < first || value.upper > last) {
            let side = if value.upper > last {
                format!("beyond its last row's, {}", shown(last))
            } else {
                format!("before its first row's, {}", shown(first))
            };
            return Err(Reason::Unreadable(format!(
                "`{column}` {} lies {side}, in table `{}`; it is not extrapolated \
                 unless `clamp` is declared",
                axis.key, self.table
            )));
        }
        let lower = value.lower.clamp(first, last);
        let upper = value.upper.clamp(first, last);
        let mut points = vec![lower];
        points.extend(
            lines
                .iter()
                .copied()
                .filter(|&line| lower < line && line < upper),
        );
        if upper > lower {
            points.push(upper);
        }
        Ok((points.into_iter().map(shown).collect(), lines))
    }

    /// The value at one key per axis (`at`), among the rows `candidates`
    /// that matched the axes before; `named` lists those axes' keys for
    /// messages.
    fn at(
        &self,
        axes: &[Axis<'_>],
        at: &[&Value],
        candidates: &[usize],
        named: &mut Vec<String>,
    ) -> Result<Value, Reason> {
        let index = named.len();
        let Some(current) = axes.get(index) else {
            return self.cell(candidates, named);
        };
        let key = at[index];
        let on = |key: &Value, named: &mut Vec<String>| {
            let matching: Vec<usize> = candidates
                .iter()
                .copied()
                .filter(|&row| same_key(&self.keys[row][index], key))
                .collect();
            named.push(format!("`{}` = {key}", current.column));
            let value = self.at(axes, at, &matching, named);
            named.pop();
            value
        };
        let (point, unit) = match key {
            Value::Number { value, unit } if current.interpolate => (value.lower, unit),
            _ => return on(key, named),
        };
        // The grid lines bracketing the key; a key on one is read there.
        // `points` keeps every key within the lines.
        let lines = &self.lines[index];
        let below = lines.iter().rev().find(|&&line| line <= point).copied();
        let above = lines.iter().find(|&&line| line >= point).copied();
        let (Some(low), Some(high)) = (below, above) else {
            return on(key, named);
        };
        #[allow(clippy::float_cmp)]
        if low == high {
            return on(key, named);
        }
        let line = |at: f64| Value::Number {
            value: Interval::point(at),
            unit: unit.clone(),
        };
        let start = on(&line(low), named)?;
        let end = on(&line(high), named)?;
        let (start, start_unit) = self.number(start)?;
        let (end, end_unit) = self.number(end)?;
        if start_unit != end_unit {
            return Err(Reason::Mismatch(format!(
                "`{}` of table `{}` holds {start_unit} and {end_unit}, which differ",
                self.column, self.table
            )));
        }
        // start + (point − low) ÷ (high − low) × (end − start)
        let (point, low, high) = (
            Interval::point(point),
            Interval::point(low),
            Interval::point(high),
        );
        let value = point
            .minus(low)
            .and_then(|offset| offset.divided_by(high.minus(low)?))
            .and_then(|share| share.times(end.minus(start)?))
            .and_then(|rise| start.plus(rise))
            .map_err(interval_reason)?;
        Ok(Value::Number {
            value,
            unit: start_unit,
        })
    }

    /// The `column` cell of the one row left at a grid point.
    fn cell(&self, candidates: &[usize], named: &[String]) -> Result<Value, Reason> {
        let at = named.join(" and ");
        match candidates {
            [] => Err(Reason::Unreadable(format!(
                "table `{}` has no row for {at}",
                self.table
            ))),
            [row] => {
                let Some(cell) = self.rows[*row].get(self.column) else {
                    return Err(Reason::Unreadable(format!(
                        "table `{}` states no `{}` for {at}",
                        self.table, self.column
                    )));
                };
                let scalar = ScalarValue::try_from(cell.clone()).map_err(|_| {
                    Reason::Mismatch(format!("column `{}` holds no single value", self.column))
                })?;
                Value::from_literal(&scalar).map_err(Reason::Mismatch)
            }
            rows => Err(Reason::Unreadable(format!(
                "rows {} of table `{}` state the same keys, {at}",
                rows.iter()
                    .map(|row| (row + 1).to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                self.table
            ))),
        }
    }

    fn number(&self, value: Value) -> Result<(Interval, Unit), Reason> {
        match value {
            Value::Number { value, unit } => Ok((value, unit)),
            other => Err(Reason::Mismatch(format!(
                "`{}` of table `{}` holds {}, which is not interpolated",
                self.column,
                self.table,
                other.kind()
            ))),
        }
    }
}

/// A cell's value, if it states one scalar.
fn cell(row: &TableRow, column: &str) -> Option<Value> {
    let scalar = ScalarValue::try_from(row.get(column)?.clone()).ok()?;
    Value::from_literal(&scalar).ok()
}

/// Whether a row's key cell is the key: one number of one unit, or the
/// same text, enumeration value or truth.
#[allow(clippy::float_cmp)]
fn same_key(cell: &Value, key: &Value) -> bool {
    match (cell, key) {
        (
            Value::Number {
                value: cell,
                unit: cell_unit,
            },
            Value::Number { value, unit },
        ) => cell_unit == unit && cell.is_point() && value.is_point() && cell.lower == value.lower,
        (Value::Text(cell) | Value::Enum(cell), Value::Text(key) | Value::Enum(key)) => cell == key,
        (cell, key) => cell == key,
    }
}

/// The one value read, or the hull of the numbers read at several points.
fn hull(values: Vec<Value>, column: &str) -> Result<Value, Reason> {
    let mut values = values.into_iter();
    let first = values.next().unwrap_or(Value::Null);
    let mut result = first.clone();
    for value in values {
        match (&mut result, value) {
            (
                Value::Number {
                    value: hull,
                    unit: own,
                },
                Value::Number { value, unit },
            ) if *own == unit => *hull = hull.hull(value),
            (_, other) => {
                return Err(Reason::Mismatch(format!(
                    "`{column}` holds {} and {}, which are not interpolated",
                    first.kind(),
                    other.kind()
                )));
            }
        }
    }
    Ok(result)
}
