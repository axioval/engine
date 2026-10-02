#![allow(clippy::doc_markdown)]

//! Spreadsheet workbooks (Office Open XML, `.xlsx`) from Axioval reports.
//!
//! An output sink, not a source adapter: it reads a finished [`Report`] and
//! the [`Project`] it was computed over, and depends on nothing else in the
//! engine. [`export`] writes one workbook:
//!
//! - the [`FINDINGS_SHEET`], one row per finding and then one per
//!   not-evaluated outcome, in report order. A workbook that listed only
//!   findings would read as "everything else passed" when it did not, so
//!   every row states its `outcome` ([`FINDING`] or [`NOT_EVALUATED`]);
//! - one sheet per [report table](axioval_ir::ReportTable), in report
//!   order (by rule, then table name).
//!
//! # Table cells
//!
//! A table sheet starts with a title row naming the rule and the table,
//! then a header row, then one row per table row. Group and text columns
//! are one text column each. A numeric column is three: `<id> lower
//! [<unit>]`, `<id> upper [<unit>]` and `<id> exactness`, the numbers
//! written as numeric cells in the column's unit:
//!
//! | Value | lower, upper | exactness |
//! |---|---|---|
//! | exact | the value twice | [`EXACT`] |
//! | interval | its bounds, never collapsed to one number | [`BOUNDED`] |
//! | unknown | blank, shaded | [`NOT_EVALUATED`] |
//!
//! An unknown text value is a blank, shaded cell, never an empty string:
//! the shading tells it from a value stated as empty.
//!
//! # Determinism
//!
//! The caller supplies the creation time ([`Options::created`]); nothing
//! reads the clock, sheet names are derived from the report alone, and the
//! archive's entries carry a fixed date, so identical input writes
//! identical bytes for a given build.

use std::collections::BTreeSet;

use axioval_ir::{
    EvidenceCheck, Finding, Location, NotEvaluated, NotEvaluatedReason, ObjectId, Project, Report,
    ReportColumn, ReportColumnKind, ReportTable, ReportValue, Scope, Severity,
};
use rust_xlsxwriter::{
    Color, DocProperties, ExcelDateTime, Format, FormatPattern, Workbook, Worksheet, XlsxError,
};
use thiserror::Error;

/// Name of the sheet listing findings and not-evaluated outcomes.
pub const FINDINGS_SHEET: &str = "Findings";
/// The `outcome` of a finding row.
pub const FINDING: &str = "finding";
/// The `outcome` of a not-evaluated row, and the exactness of an unknown
/// table value.
pub const NOT_EVALUATED: &str = "not evaluated";
/// The exactness of an exact table value.
pub const EXACT: &str = "exact";
/// The exactness of a table value known only within bounds.
pub const BOUNDED: &str = "bounded";

/// Excel's limit on the length of a sheet name, in characters.
const SHEET_NAME_LIMIT: usize = 31;
/// The widest column fitting the contents may make, in pixels.
const MAX_COLUMN_PIXELS: u32 = 480;

/// Why a workbook could not be written.
#[derive(Debug, Error)]
pub enum ExportError {
    /// The creation time is outside the range a workbook can state.
    #[error("creation time {0} s is outside 1900-01-01 to 9999-12-31")]
    Created(i64),
    /// The writer refused the workbook: more rows than a sheet holds, a
    /// text longer than a cell holds.
    #[error("{0}")]
    Write(#[from] XlsxError),
}

/// How a workbook is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// The workbook's creation time, seconds since the Unix epoch, UTC.
    pub created: i64,
    /// An external identity scheme (such as the IFC GlobalId scheme the
    /// source adapter attaches): when set, every object column is followed
    /// by one stating each object's alias in it, headed by the scheme.
    pub external_id_scheme: Option<String>,
    /// The workbook's title property.
    pub title: String,
}

impl Options {
    /// Options for a workbook created at `created`, titled `Check report`,
    /// without an external identity column.
    #[must_use]
    pub fn new(created: i64) -> Self {
        Self {
            created,
            external_id_scheme: None,
            title: "Check report".to_owned(),
        }
    }
}

/// The cell formats every sheet shares.
struct Formats {
    title: Format,
    header: Format,
    unknown: Format,
}

impl Formats {
    fn new() -> Self {
        Self {
            title: Format::new().set_bold(),
            header: Format::new()
                .set_bold()
                .set_pattern(FormatPattern::Solid)
                .set_background_color(Color::RGB(0x00DD_E4EE)),
            unknown: Format::new()
                .set_italic()
                .set_pattern(FormatPattern::Solid)
                .set_background_color(Color::RGB(0x00E0_E0E0)),
        }
    }
}

/// Writes `report` as a workbook: the [`FINDINGS_SHEET`] and one sheet per
/// report table.
///
/// # Errors
///
/// [`ExportError::Created`] for a creation time a workbook cannot state;
/// [`ExportError::Write`] when a sheet exceeds the format's limits.
pub fn export(
    report: &Report,
    project: &Project,
    options: &Options,
) -> Result<Vec<u8>, ExportError> {
    let created = ExcelDateTime::from_timestamp(options.created)
        .map_err(|_| ExportError::Created(options.created))?;
    let mut workbook = Workbook::new();
    workbook.set_properties(
        &DocProperties::new()
            .set_title(&options.title)
            .set_creation_datetime(&created),
    );
    let formats = Formats::new();
    workbook.push_worksheet(findings_sheet(report, project, options, &formats)?);
    let names = sheet_names(report.tables());
    for (table, name) in report.tables().iter().zip(names) {
        workbook.push_worksheet(table_sheet(
            report, project, options, &formats, table, &name,
        )?);
    }
    Ok(workbook.save_to_buffer()?)
}

/// Sheet names for `tables`, in order: the table's name when no other table
/// shares it, else its rule id and name, each made a valid sheet name and
/// unique (case-insensitively, as spreadsheet applications compare them)
/// among themselves and the [`FINDINGS_SHEET`].
#[must_use]
pub fn sheet_names(tables: &[ReportTable]) -> Vec<String> {
    let mut taken: BTreeSet<String> = BTreeSet::from([FINDINGS_SHEET.to_lowercase()]);
    tables
        .iter()
        .map(|table| {
            let shared = tables
                .iter()
                .filter(|other| other.name() == table.name())
                .count()
                > 1;
            let base = if shared {
                sanitize(&format!("{} {}", table.rule_id(), table.name()))
            } else {
                sanitize(table.name())
            };
            let mut name = base.clone();
            let mut n = 2;
            while taken.contains(&name.to_lowercase()) {
                let suffix = format!(" ({n})");
                let keep = SHEET_NAME_LIMIT - suffix.chars().count();
                name = base.chars().take(keep).collect::<String>() + &suffix;
                n += 1;
            }
            taken.insert(name.to_lowercase());
            name
        })
        .collect()
}

/// `name` as a sheet name: the characters a sheet name cannot hold replaced
/// by `_`, no leading or trailing apostrophe, at most 31 characters, and
/// never `History`, which spreadsheet applications reserve.
fn sanitize(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| {
            if matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = replaced.trim_matches('\'');
    let mut name: String = trimmed.chars().take(SHEET_NAME_LIMIT).collect();
    if name.is_empty() {
        "table".clone_into(&mut name);
    }
    if name.eq_ignore_ascii_case("history") {
        name.push_str(" table");
    }
    name
}

/// Writes one row of text cells starting at column 0.
fn text_row(
    sheet: &mut Worksheet,
    row: u32,
    cells: &[String],
    format: Option<&Format>,
) -> Result<(), XlsxError> {
    for (column, text) in (0u16..).zip(cells) {
        match format {
            Some(format) => sheet.write_string_with_format(row, column, text, format)?,
            None => sheet.write_string(row, column, text)?,
        };
    }
    Ok(())
}

/// Writes `text` unless it is empty, so an absent value stays a blank cell.
fn optional(sheet: &mut Worksheet, row: u32, column: u16, text: &str) -> Result<(), XlsxError> {
    if !text.is_empty() {
        sheet.write_string(row, column, text)?;
    }
    Ok(())
}

/// Freezes the rows down to `header_row`, filters the header row and
/// fits the columns.
fn finish(
    sheet: &mut Worksheet,
    header_row: u32,
    last_row: u32,
    columns: usize,
) -> Result<(), XlsxError> {
    let last_column =
        u16::try_from(columns.saturating_sub(1)).map_err(|_| XlsxError::RowColumnLimitError)?;
    sheet.set_freeze_panes(header_row + 1, 0)?;
    sheet.autofilter(header_row, 0, last_row.max(header_row), last_column)?;
    sheet.set_autofit_max_width(MAX_COLUMN_PIXELS).autofit();
    Ok(())
}

fn findings_sheet(
    report: &Report,
    project: &Project,
    options: &Options,
    formats: &Formats,
) -> Result<Worksheet, ExportError> {
    let mut sheet = Worksheet::new();
    sheet.set_name(FINDINGS_SHEET)?;
    let mut header: Vec<String> = [
        "outcome", "id", "rule", "severity", "reason", "scope", "kind",
    ]
    .map(str::to_owned)
    .to_vec();
    if let Some(scheme) = &options.external_id_scheme {
        header.push(scheme.clone());
    }
    header.extend(
        [
            "categories",
            "message",
            "related",
            "storeys",
            "spaces",
            "location unresolved",
            "decision",
            "decided by",
            "decided at",
            "assigned to",
            "due",
            "priority",
            "labels",
            "since the decision",
        ]
        .map(str::to_owned),
    );
    text_row(&mut sheet, 0, &header, Some(&formats.header))?;
    let mut row = 0;
    for finding in report.findings() {
        row += 1;
        let cells = finding_cells(report, project, options, finding);
        write_cells(&mut sheet, row, &cells)?;
    }
    for outcome in report.not_evaluated() {
        row += 1;
        let cells = not_evaluated_cells(report, project, options, outcome);
        write_cells(&mut sheet, row, &cells)?;
    }
    finish(&mut sheet, 0, row, header.len())?;
    Ok(sheet)
}

fn write_cells(sheet: &mut Worksheet, row: u32, cells: &[String]) -> Result<(), XlsxError> {
    for (column, text) in (0u16..).zip(cells) {
        optional(sheet, row, column, text)?;
    }
    Ok(())
}

/// The scope, kind and (with a scheme) external id of an outcome's subject.
fn subject(report: &Report, project: &Project, options: &Options, scope: &Scope) -> Vec<String> {
    let object = scope.object().and_then(|id| report.object(project, id));
    let mut cells = vec![
        scope.to_string(),
        object.map(|object| object.kind.clone()).unwrap_or_default(),
    ];
    if let Some(scheme) = &options.external_id_scheme {
        cells.push(
            object
                .and_then(|object| object.external_id(scheme))
                .unwrap_or_default()
                .to_owned(),
        );
    }
    cells
}

/// Object ids joined by `, `.
fn objects(ids: &[ObjectId]) -> String {
    ids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The storeys, the spaces (each by name, else by id) and why the location
/// is unresolved.
fn location(location: Option<&Location>) -> [String; 3] {
    let Some(location) = location else {
        return Default::default();
    };
    let places = |places: &[axioval_ir::Place]| {
        places
            .iter()
            .map(|place| place.name.clone().unwrap_or_else(|| place.id.to_string()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    [
        places(&location.storeys),
        places(&location.spaces),
        location.unresolved.clone().unwrap_or_default(),
    ]
}

/// `error`, `warning` or `info`.
fn severity(severity: &Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

/// The reason as written on the wire, such as `missing_service`.
fn reason(reason: &NotEvaluatedReason) -> &'static str {
    match reason {
        NotEvaluatedReason::MissingService => "missing_service",
        NotEvaluatedReason::BackendUnavailable => "backend_unavailable",
        NotEvaluatedReason::IncompleteEvidence => "incomplete_evidence",
        NotEvaluatedReason::InvalidEvidence => "invalid_evidence",
        NotEvaluatedReason::InvalidDeclaration => "invalid_declaration",
        NotEvaluatedReason::ResourceLimit => "resource_limit",
        NotEvaluatedReason::UnboundConcept => "unbound_concept",
        NotEvaluatedReason::NotRecorded => "not_recorded",
    }
}

fn finding_cells(
    report: &Report,
    project: &Project,
    options: &Options,
    finding: &Finding,
) -> Vec<String> {
    let mut cells = vec![
        FINDING.to_owned(),
        finding.id.map(|id| id.to_string()).unwrap_or_default(),
        finding.rule_id.to_string(),
        severity(&finding.severity).to_owned(),
        String::new(),
    ];
    cells.extend(subject(report, project, options, &finding.scope));
    cells.push(finding.categories.join(" / "));
    cells.push(finding.message.clone());
    cells.push(objects(&finding.related));
    cells.extend(location(finding.location.as_ref()));
    match &finding.decision {
        Some(decision) => cells.extend([
            decision.status.as_str().to_owned(),
            decision.author.clone(),
            decision.date.to_string(),
            decision.assigned_to.clone().unwrap_or_default(),
            decision
                .due_date
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            decision.priority.clone().unwrap_or_default(),
            decision.labels.join(", "),
            match decision.evidence {
                EvidenceCheck::Unchanged => "unchanged",
                EvidenceCheck::Changed => "changed",
                EvidenceCheck::Unknown => "unknown",
            }
            .to_owned(),
        ]),
        None => cells.push("undecided".to_owned()),
    }
    cells
}

fn not_evaluated_cells(
    report: &Report,
    project: &Project,
    options: &Options,
    outcome: &NotEvaluated,
) -> Vec<String> {
    let mut cells = vec![
        NOT_EVALUATED.to_owned(),
        String::new(),
        outcome.rule_id.to_string(),
        String::new(),
        reason(&outcome.reason).to_owned(),
    ];
    cells.extend(subject(report, project, options, &outcome.scope));
    cells.push(String::new());
    cells.push(outcome.message.clone());
    cells.push(String::new());
    cells.extend(location(outcome.location.as_ref()));
    cells
}

/// The header cells of one table column: one for text, three for numbers.
fn column_header(column: &ReportColumn) -> Vec<String> {
    if column.kind == ReportColumnKind::Text {
        return vec![column.id.clone()];
    }
    let unit = column
        .unit_symbol()
        .map(|unit| format!(" [{unit}]"))
        .unwrap_or_default();
    vec![
        format!("{} lower{unit}", column.id),
        format!("{} upper{unit}", column.id),
        format!("{} exactness", column.id),
    ]
}

fn table_sheet(
    report: &Report,
    project: &Project,
    options: &Options,
    formats: &Formats,
    table: &ReportTable,
    name: &str,
) -> Result<Worksheet, ExportError> {
    let mut sheet = Worksheet::new();
    sheet.set_name(name)?;
    sheet.write_string_with_format(
        0,
        0,
        format!(
            "Rule {} · table {} · shaded cells were not evaluated",
            table.rule_id(),
            table.name()
        ),
        &formats.title,
    )?;
    let mut header = vec!["scope".to_owned(), "kind".to_owned()];
    if let Some(scheme) = &options.external_id_scheme {
        header.push(scheme.clone());
    }
    header.extend(table.group_by().iter().cloned());
    for column in table.columns() {
        header.extend(column_header(column));
    }
    text_row(&mut sheet, 1, &header, Some(&formats.header))?;
    let mut row = 1;
    for table_row in table.rows() {
        row += 1;
        let mut column = 0u16;
        let mut cells = subject(report, project, options, table_row.scope());
        cells.extend(table_row.group().iter().cloned());
        for text in &cells {
            optional(&mut sheet, row, column, text)?;
            column += 1;
        }
        for (declared, value) in table.columns().iter().zip(table_row.values()) {
            let numeric = declared.kind != ReportColumnKind::Text;
            match value {
                ReportValue::Text { value } => {
                    sheet.write_string(row, column, value)?;
                    column += 1;
                }
                ReportValue::Exact { value } => {
                    sheet.write_number(row, column, *value)?;
                    sheet.write_number(row, column + 1, *value)?;
                    sheet.write_string(row, column + 2, EXACT)?;
                    column += 3;
                }
                ReportValue::Interval { lower, upper } => {
                    sheet.write_number(row, column, *lower)?;
                    sheet.write_number(row, column + 1, *upper)?;
                    sheet.write_string(row, column + 2, BOUNDED)?;
                    column += 3;
                }
                ReportValue::Unknown if numeric => {
                    sheet.write_blank(row, column, &formats.unknown)?;
                    sheet.write_blank(row, column + 1, &formats.unknown)?;
                    sheet.write_string_with_format(
                        row,
                        column + 2,
                        NOT_EVALUATED,
                        &formats.unknown,
                    )?;
                    column += 3;
                }
                ReportValue::Unknown => {
                    sheet.write_blank(row, column, &formats.unknown)?;
                    column += 1;
                }
            }
        }
    }
    finish(&mut sheet, 1, row, header.len())?;
    Ok(sheet)
}
