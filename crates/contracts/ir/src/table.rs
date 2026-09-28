//! Named tables of measured values reported beside findings.
//!
//! A finding says what is wrong; a table says what was measured, whether or
//! not it passed: one row per storey with its elevation and height, one row
//! per anchor with its areas and their ratio. Rows are keyed by the same
//! [`Scope`] as findings, so a reader joins the tables of several rules on
//! the object a row is about.
//!
//! A grouped table (a takeoff) keys its rows by a scope and a group: one
//! text value per declared group column, such as a type name and a storey,
//! so one row sums or counts every object sharing those values.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{ObjectId, QuantityDimension, RuleId, Scope, SourceId};

/// Why a report table or one of its rows was refused.
#[derive(Debug, Error, PartialEq)]
pub enum ReportTableError {
    /// A table name or column id is not a lowercase token.
    #[error(
        "invalid {kind} `{name}`: use 1 to 64 lowercase ASCII letters, digits, `-` or `_`, starting with a letter or digit"
    )]
    InvalidName { kind: &'static str, name: String },
    /// A table has no columns.
    #[error("table `{0}` has no columns")]
    NoColumns(String),
    /// Two columns share an id, group columns included.
    #[error("table `{table}` declares column `{column}` twice")]
    DuplicateColumn { table: String, column: String },
    /// A row repeats a scope and group, has not one group value per group
    /// column or not one value per column, or holds a
    /// value that does not fit its column: text in a numeric column, a
    /// number in a text column, a non-finite number, or an interval whose
    /// lower bound is not below its upper bound.
    #[error("table `{table}` row {row}: {detail}")]
    InvalidRow {
        table: String,
        /// The row's scope, as displayed.
        row: String,
        detail: String,
    },
}

/// What a column holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportColumnKind {
    /// A quantity in the coherent SI unit of its dimension (metres, square
    /// metres, ...), as property quantities are.
    Quantity { dimension: QuantityDimension },
    /// A dimensionless number, such as a ratio.
    Number,
    /// Text, such as the name of a related object.
    Text,
}

impl ReportColumnKind {
    /// The unit symbol values of this column are stated in, if any.
    #[must_use]
    pub fn unit_symbol(self) -> Option<String> {
        match self {
            Self::Quantity { dimension } => Some(dimension.unit_symbol()),
            Self::Number | Self::Text => None,
        }
    }
}

/// One column: a lowercase id, unique in its table, and its kind.
///
/// On the wire `{"id": "height", "kind": "quantity", "dimension": "length"}`;
/// `dimension` is written for a quantity column only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ColumnWire", into = "ColumnWire")]
pub struct ReportColumn {
    pub id: String,
    pub kind: ReportColumnKind,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum KindTag {
    Quantity,
    Number,
    Text,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ColumnWire {
    id: String,
    kind: KindTag,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dimension: Option<QuantityDimension>,
}

impl From<ReportColumn> for ColumnWire {
    fn from(column: ReportColumn) -> Self {
        let (kind, dimension) = match column.kind {
            ReportColumnKind::Quantity { dimension } => (KindTag::Quantity, Some(dimension)),
            ReportColumnKind::Number => (KindTag::Number, None),
            ReportColumnKind::Text => (KindTag::Text, None),
        };
        Self {
            id: column.id,
            kind,
            dimension,
        }
    }
}

impl TryFrom<ColumnWire> for ReportColumn {
    type Error = String;
    fn try_from(wire: ColumnWire) -> Result<Self, String> {
        let kind = match (wire.kind, wire.dimension) {
            (KindTag::Quantity, Some(dimension)) => ReportColumnKind::Quantity { dimension },
            (KindTag::Number, None) => ReportColumnKind::Number,
            (KindTag::Text, None) => ReportColumnKind::Text,
            (KindTag::Quantity, None) => {
                return Err(format!("quantity column `{}` has no dimension", wire.id));
            }
            (_, Some(_)) => {
                return Err(format!(
                    "column `{}` states a dimension but is not a quantity",
                    wire.id
                ));
            }
        };
        Ok(Self { id: wire.id, kind })
    }
}

impl ReportColumn {
    /// A quantity column in the coherent SI unit of `dimension`.
    #[must_use]
    pub fn quantity(id: impl Into<String>, dimension: QuantityDimension) -> Self {
        Self {
            id: id.into(),
            kind: ReportColumnKind::Quantity { dimension },
        }
    }
    /// A dimensionless number column.
    #[must_use]
    pub fn number(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: ReportColumnKind::Number,
        }
    }
    /// A text column.
    #[must_use]
    pub fn text(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: ReportColumnKind::Text,
        }
    }
}

/// One cell.
///
/// A measured value is exact only when it is a point: a bound measured
/// from geometry is an interval sure to hold the exact value, never a point
/// claimed from a sample.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReportValue {
    /// Not measured, or not measurable to finite bounds. The rule's
    /// not-evaluated outcomes say why.
    Unknown,
    /// An exact number.
    Exact { value: f64 },
    /// A number known to lie within `lower..=upper`, `lower < upper`.
    Interval { lower: f64, upper: f64 },
    /// Text.
    Text { value: String },
}

impl ReportValue {
    /// An exact number, or [`Self::Unknown`] when it is not finite.
    #[must_use]
    pub fn exact(value: f64) -> Self {
        Self::measured(value, value)
    }
    /// A measured interval: [`Self::Exact`] when it is a point, and
    /// [`Self::Unknown`] when a bound is not finite or the bounds are
    /// reversed.
    #[must_use]
    pub fn measured(lower: f64, upper: f64) -> Self {
        if !lower.is_finite() || !upper.is_finite() || lower > upper {
            Self::Unknown
        } else if lower < upper {
            Self::Interval { lower, upper }
        } else {
            Self::Exact { value: lower }
        }
    }
    /// Text.
    #[must_use]
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text {
            value: value.into(),
        }
    }

    /// Why this value does not fit a column of `kind`, if it does not.
    fn misfit(&self, kind: ReportColumnKind) -> Option<&'static str> {
        let text = kind == ReportColumnKind::Text;
        match self {
            Self::Unknown => None,
            Self::Text { .. } => (!text).then_some("text in a numeric column"),
            Self::Exact { .. } | Self::Interval { .. } if text => Some("a number in a text column"),
            Self::Exact { value } => (!value.is_finite()).then_some("the value is not finite"),
            Self::Interval { lower, upper } => {
                if !lower.is_finite() || !upper.is_finite() {
                    Some("an interval bound is not finite")
                } else if lower >= upper {
                    Some(
                        "an interval's lower bound must lie below its upper bound; a point is exact",
                    )
                } else {
                    None
                }
            }
        }
    }
}

impl fmt::Display for ReportValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => f.write_str("unknown"),
            Self::Exact { value } => write!(f, "{value}"),
            Self::Interval { lower, upper } => write!(f, "{lower}..{upper}"),
            Self::Text { value } => f.write_str(value),
        }
    }
}

/// One row: what it is about, and one value per column.
#[derive(Clone, Debug, PartialEq)]
pub struct ReportRow {
    scope: Scope,
    group: Vec<String>,
    values: Vec<ReportValue>,
}

impl ReportRow {
    /// What the row is about: an object, a source or the project; in a
    /// grouped table, the scope its group was formed in.
    #[must_use]
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    /// One value per group column, in group column order; empty unless the
    /// table is grouped.
    #[must_use]
    pub fn group(&self) -> &[String] {
        &self.group
    }
    fn key(&self) -> (&Scope, &[String]) {
        (&self.scope, &self.group)
    }
    /// One value per column, in column order.
    #[must_use]
    pub fn values(&self) -> &[ReportValue] {
        &self.values
    }
}

/// A named table of measured values, reported by one rule.
///
/// Rows are keyed by scope, at most one per scope, and kept in scope order
/// (project, then sources, then objects, each by identity), whatever order
/// they were added or read in. A grouped table declares group columns and
/// keys each row by its scope and its group, one text value per group
/// column, ordered by scope and then by group. Construction and
/// deserialization validate every name, row width and value.
///
/// On the wire a row names its scope like a finding: `object_id` for an
/// object, `source` for a source, neither for the project. A grouped table
/// writes its group column ids as `group_by` and each row's values as
/// `group`; an ungrouped table writes neither, as before groups existed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "TableWire", into = "TableWire")]
pub struct ReportTable {
    rule_id: RuleId,
    name: String,
    group_by: Vec<String>,
    columns: Vec<ReportColumn>,
    rows: Vec<ReportRow>,
}

impl ReportTable {
    /// An empty table of `rule_id` named `name` with `columns`.
    pub fn new(
        rule_id: RuleId,
        name: impl Into<String>,
        columns: Vec<ReportColumn>,
    ) -> Result<Self, ReportTableError> {
        Self::grouped(rule_id, name, Vec::new(), columns)
    }

    /// An empty table of `rule_id` named `name` with `columns`, its rows
    /// keyed by scope and by one text value per id of `group_by`.
    ///
    /// Without group columns the table is keyed by scope alone, as
    /// [`Self::new`] makes it. Group column ids are tokens like column ids
    /// and differ from one another and from every column id.
    pub fn grouped(
        rule_id: RuleId,
        name: impl Into<String>,
        group_by: Vec<String>,
        columns: Vec<ReportColumn>,
    ) -> Result<Self, ReportTableError> {
        let name = token(name.into(), "table name")?;
        if columns.is_empty() {
            return Err(ReportTableError::NoColumns(name));
        }
        let mut columns_seen = std::collections::BTreeSet::new();
        for id in group_by
            .iter()
            .chain(columns.iter().map(|column| &column.id))
        {
            token(id.clone(), "column id")?;
            if !columns_seen.insert(id.as_str()) {
                return Err(ReportTableError::DuplicateColumn {
                    table: name,
                    column: id.clone(),
                });
            }
        }
        Ok(Self {
            rule_id,
            name,
            group_by,
            columns,
            rows: Vec::new(),
        })
    }

    /// Adds the row about `scope`, keeping rows in scope order.
    ///
    /// Refused, leaving the table unchanged, when a row about `scope`
    /// exists, the table is grouped, or the values do not fit the columns.
    pub fn push_row(
        &mut self,
        scope: impl Into<Scope>,
        values: Vec<ReportValue>,
    ) -> Result<(), ReportTableError> {
        self.push_group_row(scope, Vec::new(), values)
    }

    /// Adds the row of `group` in `scope`, keeping rows in scope and group
    /// order.
    ///
    /// Refused, leaving the table unchanged, when that row exists, `group`
    /// has not one value per group column, or the values do not fit the
    /// columns.
    pub fn push_group_row(
        &mut self,
        scope: impl Into<Scope>,
        group: Vec<String>,
        values: Vec<ReportValue>,
    ) -> Result<(), ReportTableError> {
        let scope = scope.into();
        let refuse = |detail: String| ReportTableError::InvalidRow {
            table: self.name.clone(),
            row: if group.is_empty() {
                scope.to_string()
            } else {
                format!("{scope} [{}]", group.join("] ["))
            },
            detail,
        };
        if group.len() != self.group_by.len() {
            return Err(refuse(format!(
                "{} group value(s) for {} group column(s)",
                group.len(),
                self.group_by.len()
            )));
        }
        if values.len() != self.columns.len() {
            return Err(refuse(format!(
                "{} value(s) for {} column(s)",
                values.len(),
                self.columns.len()
            )));
        }
        for (value, column) in values.iter().zip(&self.columns) {
            if let Some(detail) = value.misfit(column.kind) {
                return Err(refuse(format!("column `{}`: {detail}", column.id)));
            }
        }
        match self
            .rows
            .binary_search_by(|row| row.key().cmp(&(&scope, group.as_slice())))
        {
            Ok(_) => Err(refuse("a row about it exists already".into())),
            Err(at) => {
                self.rows.insert(
                    at,
                    ReportRow {
                        scope,
                        group,
                        values,
                    },
                );
                Ok(())
            }
        }
    }

    /// The same table reported by `rule_id`, as the runtime binds a
    /// capability's table to the compiled rule.
    #[must_use]
    pub fn with_rule_id(mut self, rule_id: RuleId) -> Self {
        self.rule_id = rule_id;
        self
    }
    /// The rule that reported the table.
    #[must_use]
    pub fn rule_id(&self) -> &RuleId {
        &self.rule_id
    }
    /// The table's name, unique among the tables of its rule.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// The group column ids, in order; empty unless the table is grouped.
    #[must_use]
    pub fn group_by(&self) -> &[String] {
        &self.group_by
    }
    #[must_use]
    pub fn columns(&self) -> &[ReportColumn] {
        &self.columns
    }
    /// Rows in scope order, and by group within one scope.
    #[must_use]
    pub fn rows(&self) -> &[ReportRow] {
        &self.rows
    }
    /// The row about `scope`, if there is one; a grouped table has none.
    #[must_use]
    pub fn row(&self, scope: &Scope) -> Option<&ReportRow> {
        self.group_row(scope, &[])
    }
    /// The row of `group` in `scope`, if there is one.
    #[must_use]
    pub fn group_row(&self, scope: &Scope, group: &[String]) -> Option<&ReportRow> {
        self.rows
            .binary_search_by(|row| row.key().cmp(&(scope, group)))
            .ok()
            .map(|at| &self.rows[at])
    }
    /// Whether the table has no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// A lowercase token of 1 to 64 ASCII letters, digits, `-` or `_`,
/// starting with a letter or digit.
fn token(name: String, kind: &'static str) -> Result<String, ReportTableError> {
    let valid = (1..=64).contains(&name.len())
        && name
            .bytes()
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_".contains(&byte)
        });
    if valid {
        Ok(name)
    } else {
        Err(ReportTableError::InvalidName { kind, name })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TableWire {
    rule_id: RuleId,
    name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    group_by: Vec<String>,
    columns: Vec<ReportColumn>,
    rows: Vec<RowWire>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RowWire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    object_id: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<SourceId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    group: Vec<String>,
    values: Vec<ReportValue>,
}

impl From<ReportTable> for TableWire {
    fn from(table: ReportTable) -> Self {
        Self {
            rule_id: table.rule_id,
            name: table.name,
            group_by: table.group_by,
            columns: table.columns,
            rows: table
                .rows
                .into_iter()
                .map(|row| {
                    let (object_id, source) = row.scope.into_wire();
                    RowWire {
                        object_id,
                        source,
                        group: row.group,
                        values: row.values,
                    }
                })
                .collect(),
        }
    }
}

impl TryFrom<TableWire> for ReportTable {
    type Error = String;
    fn try_from(wire: TableWire) -> Result<Self, String> {
        let mut table = Self::grouped(wire.rule_id, wire.name, wire.group_by, wire.columns)
            .map_err(|error| error.to_string())?;
        for row in wire.rows {
            let scope = Scope::from_wire(row.object_id, row.source)?;
            table
                .push_group_row(scope, row.group, row.values)
                .map_err(|error| error.to_string())?;
        }
        Ok(table)
    }
}
