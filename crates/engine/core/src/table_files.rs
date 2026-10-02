//! Rule parameter tables read from data files shipped in a package.
//!
//! A `table` parameter may name a data file instead of listing its rows
//! ([`ParameterValue::TableFile`]): a CSV file, or one named sheet of an
//! xlsx workbook, with its columns declared beside the reference. A host
//! loads every such file before binding ([`load_table_files`]); the binder
//! then checks the declared columns against the parameter's table and binds
//! the rows exactly as it binds the same rows written inline.
//!
//! The file is data, never code. Loading is deterministic and fails closed:
//! a path outside the package, a digest other than the declared one, a
//! header missing a declared column or naming an undeclared one, and a cell
//! that is not of its column's kind are refused, and nothing is guessed. A
//! workbook cell holding a formula is refused rather than read from its
//! cached result.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use axioval_ir::contract::{
    ColumnKind, DefinitionPackage, ParameterValue, RelationKey, RuleFolder, RuleSetPackage,
    TableFileColumn, TableFileReference, TableRow,
};
use quick_xml::XmlVersion;
use quick_xml::events::Event;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// The largest table file read, and the largest member of a workbook
/// unpacked, in bytes.
pub const TABLE_FILE_LIMIT_BYTES: u64 = 10_000_000;

/// The files of one package, by path relative to its root.
pub trait PackageFiles {
    /// The bytes of the file at `path`, a checked relative path (see
    /// [`load_table_files`]).
    ///
    /// # Errors
    ///
    /// Why the file cannot be read.
    fn read(&self, path: &str) -> Result<Vec<u8>, String>;
}

/// The files of a package unpacked in a directory.
///
/// A path resolving outside the directory (through a link), a path that is
/// not a regular file and a file larger than [`TABLE_FILE_LIMIT_BYTES`] are
/// refused.
#[derive(Clone, Debug)]
pub struct PackageDirectory {
    root: PathBuf,
}

impl PackageDirectory {
    /// The package whose root is `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl PackageFiles for PackageDirectory {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        let root = self
            .root
            .canonicalize()
            .map_err(|error| format!("the package root cannot be read: {error}"))?;
        let file = root
            .join(Path::new(path))
            .canonicalize()
            .map_err(|error| format!("the file cannot be read: {error}"))?;
        if !file.starts_with(&root) {
            return Err("the file lies outside the package".into());
        }
        let metadata = file
            .metadata()
            .map_err(|error| format!("the file cannot be read: {error}"))?;
        if !metadata.is_file() {
            return Err("not a regular file".into());
        }
        if metadata.len() > TABLE_FILE_LIMIT_BYTES {
            return Err(format!(
                "the file is larger than {TABLE_FILE_LIMIT_BYTES} bytes"
            ));
        }
        std::fs::read(&file).map_err(|error| format!("the file cannot be read: {error}"))
    }
}

/// Why a table file was refused.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("{location}: table file `{path}`: {detail}")]
pub struct TableFileError {
    /// Where the reference is: a rule's or definition's parameter.
    pub location: String,
    pub path: String,
    pub detail: String,
}

/// Loads every table file the rules of `ruleset` name, from `files`.
///
/// # Errors
///
/// The first reference refused, in rule and parameter order.
pub fn load_table_files(
    ruleset: &mut RuleSetPackage,
    files: &dyn PackageFiles,
) -> Result<(), TableFileError> {
    load_folder(&mut ruleset.root, files)?;
    for (id, relation) in &mut ruleset.relations {
        if let RelationKey::Pairs { pairs, .. } = &mut relation.by {
            load_value(pairs, files, &format!("relation `{id}` pairs"))?;
        }
    }
    Ok(())
}

/// Loads every table file the parameter defaults of `package` name, from
/// `files`.
///
/// # Errors
///
/// The first reference refused, in definition and parameter order.
pub fn load_definition_table_files(
    package: &mut DefinitionPackage,
    files: &dyn PackageFiles,
) -> Result<(), TableFileError> {
    for (id, definition) in &mut package.definitions {
        for (name, parameter) in &mut definition.parameters {
            if let Some(value) = &mut parameter.default_value {
                load_value(
                    value,
                    files,
                    &format!("definition `{id}` parameter `{name}`"),
                )?;
            }
        }
    }
    Ok(())
}

fn load_folder(folder: &mut RuleFolder, files: &dyn PackageFiles) -> Result<(), TableFileError> {
    for rule in &mut folder.rules {
        for (name, value) in &mut rule.parameters {
            load_value(
                value,
                files,
                &format!("rule `{}` parameter `{name}`", rule.id),
            )?;
        }
    }
    folder
        .folders
        .iter_mut()
        .try_for_each(|child| load_folder(child, files))
}

fn load_value(
    value: &mut ParameterValue,
    files: &dyn PackageFiles,
    location: &str,
) -> Result<(), TableFileError> {
    let ParameterValue::TableFile(file) = value else {
        return Ok(());
    };
    let TableFileReference {
        path,
        sheet,
        sha256,
        columns,
        rows,
    } = &mut **file;
    let refuse = |detail: String| TableFileError {
        location: location.to_owned(),
        path: path.clone(),
        detail,
    };
    check_path(path).map_err(refuse)?;
    let bytes = files.read(path).map_err(refuse)?;
    let digest = Sha256::digest(&bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        });
    if *sha256 != digest {
        return Err(refuse(format!(
            "the file's SHA-256 is {digest}, not the declared {sha256}"
        )));
    }
    *rows = Some(read_table_file(&bytes, path, sheet.as_deref(), columns).map_err(refuse)?);
    Ok(())
}

/// A relative, `/`-separated path without empty, `.` or `..` segments.
fn check_path(path: &str) -> Result<(), String> {
    let unsafe_path = path.is_empty()
        || path.starts_with('/')
        || path.contains(['\\', ':', '\0'])
        || path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..");
    if unsafe_path {
        Err("the path must be relative to the package root, `/`-separated, without empty, `.` or `..` segments".into())
    } else {
        Ok(())
    }
}

/// The rows of the table file `path` holds as `bytes`: a `.csv` file, or
/// the sheet `sheet` of an `.xlsx` workbook.
///
/// The first row that is not blank is the header; every header names a
/// declared column (by its `header`, its id when omitted), and every
/// declared column is named once. Every later row that is not blank is a
/// table row: an empty cell leaves its column out of the row, any other
/// cell is read as its column's kind (a `number` or `quantity` as a finite
/// decimal number, the unit of a `quantity` from its declaration; an
/// `integer` as a whole number; a `boolean` as `true` or `false`; a `date`
/// or `dateTime` as an ISO 8601 literal; any other kind as its text).
///
/// # Errors
///
/// Why the file or its columns were refused.
pub fn read_table_file(
    bytes: &[u8],
    path: &str,
    sheet: Option<&str>,
    columns: &[TableFileColumn],
) -> Result<Vec<TableRow>, String> {
    check_columns(columns)?;
    let grid = match (path.rsplit_once('.').map(|(_, extension)| extension), sheet) {
        (Some("csv"), None) => csv_rows(bytes)?,
        (Some("csv"), Some(_)) => return Err("a CSV file has no sheets; omit `sheet`".into()),
        (Some("xlsx"), Some(sheet)) => xlsx_rows(bytes, sheet)?,
        (Some("xlsx"), None) => return Err("name the workbook's `sheet` to read".into()),
        _ => return Err("the file is neither a `.csv` file nor an `.xlsx` workbook".into()),
    };
    let mut rows = grid
        .into_iter()
        .filter(|(_, cells)| cells.iter().any(|cell| !cell.is_empty()));
    let Some((_, header)) = rows.next() else {
        return Err("the file has no header row".into());
    };
    let mut order = Vec::with_capacity(header.len());
    let mut seen = BTreeSet::new();
    for name in &header {
        if !seen.insert(name.as_str()) {
            return Err(format!("the header names column `{name}` twice"));
        }
        let column = columns
            .iter()
            .find(|column| column.header() == name)
            .ok_or_else(|| format!("the header names column `{name}`, which is not declared"))?;
        order.push(column);
    }
    if let Some(missing) = columns
        .iter()
        .find(|column| !seen.contains(column.header()))
    {
        return Err(format!(
            "the declared column `{}` is missing from the header",
            missing.header()
        ));
    }
    rows.map(|(number, cells)| {
        if cells.len() > order.len() {
            return Err(format!(
                "row {number} has {} cells; the header has {}",
                cells.len(),
                order.len()
            ));
        }
        let mut row = TableRow::new();
        for (cell, column) in cells.iter().zip(&order) {
            if cell.is_empty() {
                continue;
            }
            let value = read_cell(cell, column)
                .map_err(|detail| format!("row {number} column `{}`: {detail}", column.header()))?;
            row.insert(column.id.clone(), value);
        }
        Ok(row)
    })
    .collect()
}

/// Ids and headers distinct, no selector column, a unit exactly for every
/// quantity column.
fn check_columns(columns: &[TableFileColumn]) -> Result<(), String> {
    if columns.is_empty() {
        return Err("no columns are declared".into());
    }
    let mut ids = BTreeSet::new();
    let mut headers = BTreeSet::new();
    for column in columns {
        if !ids.insert(column.id.as_str()) {
            return Err(format!("column `{}` is declared twice", column.id));
        }
        if !headers.insert(column.header()) {
            return Err(format!("two columns are headed `{}`", column.header()));
        }
        if column.header().is_empty() {
            return Err(format!("column `{}` has an empty header", column.id));
        }
        match (column.kind, column.unit.as_deref()) {
            (ColumnKind::Selector, _) => {
                return Err(format!(
                    "column `{}` is a selector column, which a file cannot hold",
                    column.id
                ));
            }
            (ColumnKind::Quantity, None) => {
                return Err(format!("quantity column `{}` declares no unit", column.id));
            }
            (ColumnKind::Quantity, Some(unit)) if unit.trim().is_empty() => {
                return Err(format!("quantity column `{}` has a blank unit", column.id));
            }
            (ColumnKind::Quantity, Some(_)) | (_, None) => {}
            (_, Some(_)) => {
                return Err(format!(
                    "column `{}` declares a unit but is not a quantity column",
                    column.id
                ));
            }
        }
    }
    Ok(())
}

fn read_cell(cell: &str, column: &TableFileColumn) -> Result<ParameterValue, String> {
    let number = || {
        cell.parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("`{cell}` is not a number"))
    };
    Ok(match column.kind {
        ColumnKind::String | ColumnKind::TextPattern => ParameterValue::String {
            value: cell.to_owned(),
        },
        ColumnKind::Reference => ParameterValue::Reference {
            value: cell.to_owned(),
        },
        ColumnKind::Number => ParameterValue::Number { value: number()? },
        ColumnKind::Quantity => ParameterValue::Quantity {
            value: number()?,
            unit: column.unit.clone().unwrap_or_default(),
        },
        ColumnKind::Integer => ParameterValue::Integer {
            value: cell
                .parse()
                .map_err(|_| format!("`{cell}` is not a whole number"))?,
        },
        ColumnKind::Boolean => ParameterValue::Boolean {
            value: match cell {
                "true" => true,
                "false" => false,
                _ => return Err(format!("`{cell}` is neither `true` nor `false`")),
            },
        },
        ColumnKind::Date => ParameterValue::Date {
            value: cell.parse().map_err(|error| format!("{error}"))?,
        },
        ColumnKind::DateTime => ParameterValue::DateTime {
            value: cell.parse().map_err(|error| format!("{error}"))?,
        },
        ColumnKind::Selector => return Err("a selector cannot be read from a file".into()),
    })
}

/// Rows of cells with their one-based row numbers.
type Grid = Vec<(usize, Vec<String>)>;

/// The records of an RFC 4180 CSV file: UTF-8 (a leading byte order mark
/// is dropped), comma-separated, fields optionally double-quoted with `""`
/// for a quote, records ended by CRLF or LF. Every record has as many
/// fields as the header.
fn csv_rows(bytes: &[u8]) -> Result<Grid, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "the file is not UTF-8 text".to_owned())?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut records: Grid = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let (mut quoted, mut closed, mut started) = (false, false, false);
    let mut chars = text.chars().peekable();
    let end = |records: &mut Grid, record: &mut Vec<String>, field: &mut String| {
        record.push(std::mem::take(field));
        let number = records.len() + 1;
        records.push((number, std::mem::take(record)));
    };
    while let Some(c) = chars.next() {
        let number = records.len() + 1;
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                    closed = true;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if !started => {
                quoted = true;
                started = true;
            }
            ',' => {
                record.push(std::mem::take(&mut field));
                (closed, started) = (false, false);
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                end(&mut records, &mut record, &mut field);
                (closed, started) = (false, false);
            }
            '"' => return Err(format!("row {number}: a quote inside an unquoted field")),
            _ if closed => return Err(format!("row {number}: text after a closing quote")),
            _ => {
                field.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return Err(format!(
            "row {}: a quoted field is not closed",
            records.len() + 1
        ));
    }
    if started || !record.is_empty() {
        end(&mut records, &mut record, &mut field);
    }
    let width = records
        .iter()
        .find(|(_, cells)| cells.iter().any(|cell| !cell.is_empty()))
        .map_or(0, |(_, cells)| cells.len());
    if let Some((number, cells)) = records
        .iter()
        .find(|(_, cells)| cells.len() != width && cells.iter().any(|cell| !cell.is_empty()))
    {
        return Err(format!(
            "row {number} has {} fields; the header has {width}",
            cells.len()
        ));
    }
    Ok(records)
}

/// The rows of the sheet `sheet` of an xlsx workbook, each cell as text: a
/// shared or inline string as written, a number as its literal, a boolean
/// as `true` or `false`. A formula or an error cell is refused.
fn xlsx_rows(bytes: &[u8], sheet: &str) -> Result<Grid, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("the workbook cannot be read: {error}"))?;
    let mut member = |name: &str| -> Result<Option<String>, String> {
        let file = match archive.by_name(name) {
            Ok(file) => file,
            Err(zip::result::ZipError::FileNotFound) => return Ok(None),
            Err(error) => return Err(format!("workbook member `{name}`: {error}")),
        };
        let mut text = String::new();
        file.take(TABLE_FILE_LIMIT_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(|error| format!("workbook member `{name}`: {error}"))?;
        if text.len() as u64 > TABLE_FILE_LIMIT_BYTES {
            return Err(format!(
                "workbook member `{name}` unpacks to more than {TABLE_FILE_LIMIT_BYTES} bytes"
            ));
        }
        Ok(Some(text))
    };
    let required = |text: Option<String>, name: &str| {
        text.ok_or_else(|| format!("the workbook has no `{name}`"))
    };
    let workbook = required(member("xl/workbook.xml")?, "xl/workbook.xml")?;
    let relationship = sheet_relationship(&workbook, sheet)?;
    let relationships = required(
        member("xl/_rels/workbook.xml.rels")?,
        "xl/_rels/workbook.xml.rels",
    )?;
    let target = relationship_target(&relationships, &relationship)?;
    let target = match target.strip_prefix('/') {
        Some(absolute) => absolute.to_owned(),
        None => format!("xl/{target}"),
    };
    let shared = match member("xl/sharedStrings.xml")? {
        Some(xml) => shared_strings(&xml)?,
        None => Vec::new(),
    };
    let worksheet = required(member(&target)?, &target)?;
    sheet_cells(&worksheet, &shared)
}

/// One XML event, by local name.
enum Xml {
    Open(String, BTreeMap<String, String>),
    Close(String),
    Text(String),
}

/// Walks `xml`, calling `on` for each element and text; an empty element
/// opens and closes. A document type, a processing instruction or an
/// entity other than the predefined ones is refused.
fn walk_xml(xml: &str, mut on: impl FnMut(Xml) -> Result<(), String>) -> Result<(), String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let malformed = |error: &dyn std::fmt::Display| format!("malformed workbook XML: {error}");
    loop {
        let event = reader.read_event().map_err(|error| malformed(&error))?;
        let (element, empty) = match event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(element) => {
                on(Xml::Close(local(element.name().as_ref()).to_owned()))?;
                continue;
            }
            other => {
                if walk_other(other, &mut on)? {
                    return Ok(());
                }
                continue;
            }
        };
        let name = local(element.name().as_ref()).to_owned();
        let mut attributes = BTreeMap::new();
        for attribute in element.attributes() {
            let attribute = attribute.map_err(|error| malformed(&error))?;
            let value = attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|error| malformed(&error))?;
            attributes.insert(local(attribute.key.as_ref()).to_owned(), value.into_owned());
        }
        on(Xml::Open(name.clone(), attributes))?;
        if empty {
            on(Xml::Close(name))?;
        }
    }
}

/// Passes on text and references; `true` at the end of the document.
fn walk_other(
    event: Event<'_>,
    on: &mut impl FnMut(Xml) -> Result<(), String>,
) -> Result<bool, String> {
    let malformed = |error: &dyn std::fmt::Display| format!("malformed workbook XML: {error}");
    {
        match event {
            Event::Text(text) => on(Xml::Text(
                text.xml_content(XmlVersion::Implicit1_0).into_owned(),
            ))?,
            Event::CData(data) => on(Xml::Text(
                data.xml_content(XmlVersion::Implicit1_0).into_owned(),
            ))?,
            Event::GeneralRef(reference) => {
                let resolved = match reference.resolve_char_ref() {
                    Ok(Some(c)) => c.to_string(),
                    Ok(None) => {
                        let name = reference.xml_content(XmlVersion::Implicit1_0);
                        quick_xml::escape::resolve_predefined_entity(&name)
                            .ok_or_else(|| format!("malformed workbook XML: entity `{name}`"))?
                            .to_owned()
                    }
                    Err(error) => return Err(malformed(&error)),
                };
                on(Xml::Text(resolved))?;
            }
            Event::DocType(_) | Event::PI(_) => {
                return Err(
                    "workbook XML declares a document type or processing instruction".into(),
                );
            }
            Event::Eof => return Ok(true),
            _ => {}
        }
    }
    Ok(false)
}

fn local(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

/// The relationship id of the sheet named `sheet`.
fn sheet_relationship(workbook: &str, sheet: &str) -> Result<String, String> {
    let mut found = None;
    let mut names = Vec::new();
    walk_xml(workbook, |event| {
        if let Xml::Open(name, attributes) = event
            && name == "sheet"
        {
            let label = attributes.get("name").cloned().unwrap_or_default();
            if label == sheet {
                found = attributes.get("id").cloned();
            }
            names.push(label);
        }
        Ok(())
    })?;
    found.ok_or_else(|| {
        format!(
            "the workbook has no sheet `{sheet}` (its sheets: {})",
            names.join(", ")
        )
    })
}

/// The target of the relationship `id`.
fn relationship_target(relationships: &str, id: &str) -> Result<String, String> {
    let mut found = None;
    walk_xml(relationships, |event| {
        if let Xml::Open(name, attributes) = event
            && name == "Relationship"
            && attributes.get("Id").map(String::as_str) == Some(id)
        {
            found = attributes.get("Target").cloned();
        }
        Ok(())
    })?;
    let target = found.ok_or_else(|| format!("the workbook has no relationship `{id}`"))?;
    check_path(target.trim_start_matches('/'))
        .map(|()| target.clone())
        .map_err(|_| format!("the workbook's sheet lies at `{target}`, outside it"))
}

/// The shared strings, each the text of its runs (phonetic runs left out).
fn shared_strings(xml: &str) -> Result<Vec<String>, String> {
    let mut strings = Vec::new();
    let (mut item, mut phonetic, mut text) = (None::<String>, 0_usize, false);
    walk_xml(xml, |event| {
        match event {
            Xml::Open(name, _) => match name.as_str() {
                "si" => item = Some(String::new()),
                "rPh" => phonetic += 1,
                "t" => text = true,
                _ => {}
            },
            Xml::Close(name) => match name.as_str() {
                "si" => strings.push(item.take().unwrap_or_default()),
                "rPh" => phonetic = phonetic.saturating_sub(1),
                "t" => text = false,
                _ => {}
            },
            Xml::Text(content) => {
                if let Some(item) = item.as_mut().filter(|_| text && phonetic == 0) {
                    item.push_str(&content);
                }
            }
        }
        Ok(())
    })?;
    Ok(strings)
}

/// One cell being read.
#[derive(Default)]
struct Cell {
    kind: String,
    column: usize,
    value: Option<String>,
    inline: Option<String>,
    formula: bool,
}

/// The rows of a worksheet, by row number, each cell as text.
fn sheet_cells(xml: &str, shared: &[String]) -> Result<Grid, String> {
    let mut rows: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    let mut row_number = 0_usize;
    let mut cell: Option<Cell> = None;
    let mut reading: Option<&'static str> = None;
    let mut next_column = 0_usize;
    walk_xml(xml, |event| {
        match event {
            Xml::Open(name, attributes) => match name.as_str() {
                "row" => {
                    row_number = match attributes.get("r") {
                        Some(number) => number
                            .parse()
                            .ok()
                            .filter(|number| *number > 0)
                            .ok_or_else(|| format!("row number `{number}` is not a row"))?,
                        None => row_number + 1,
                    };
                    next_column = 0;
                }
                "c" => {
                    let column = match attributes.get("r") {
                        Some(reference) => column_index(reference)?,
                        None => next_column,
                    };
                    next_column = column + 1;
                    cell = Some(Cell {
                        kind: attributes.get("t").cloned().unwrap_or_else(|| "n".into()),
                        column,
                        ..Cell::default()
                    });
                }
                "v" => reading = Some("v"),
                "t" if cell.is_some() => reading = Some("t"),
                "f" => {
                    if let Some(cell) = cell.as_mut() {
                        cell.formula = true;
                    }
                }
                _ => {}
            },
            Xml::Text(text) => {
                if let (Some(cell), Some(slot)) = (cell.as_mut(), reading) {
                    let target = if slot == "v" {
                        &mut cell.value
                    } else {
                        &mut cell.inline
                    };
                    target.get_or_insert_with(String::new).push_str(&text);
                }
            }
            Xml::Close(name) => match name.as_str() {
                "v" | "t" => reading = None,
                "c" => {
                    let Some(done) = cell.take() else {
                        return Ok(());
                    };
                    let reference = format!("row {row_number} column {}", done.column + 1);
                    let text = cell_text(done.kind.as_str(), &done, shared)
                        .map_err(|detail| format!("{reference}: {detail}"))?;
                    let row = rows.entry(row_number.max(1)).or_default();
                    if row.len() <= done.column {
                        row.resize(done.column + 1, String::new());
                    }
                    row[done.column] = text;
                }
                _ => {}
            },
        }
        Ok(())
    })?;
    Ok(rows.into_iter().collect())
}

fn cell_text(kind: &str, cell: &Cell, shared: &[String]) -> Result<String, String> {
    if cell.formula {
        return Err("a formula; store its value instead".into());
    }
    let value = cell.value.clone().unwrap_or_default();
    match kind {
        "s" if value.is_empty() => Ok(String::new()),
        "s" => value
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|index| shared.get(index))
            .cloned()
            .ok_or_else(|| format!("shared string `{value}` does not exist")),
        "inlineStr" => Ok(cell.inline.clone().unwrap_or_default()),
        "str" | "d" | "n" => Ok(value),
        "b" => match value.as_str() {
            "1" => Ok("true".into()),
            "0" => Ok("false".into()),
            "" => Ok(String::new()),
            _ => Err(format!("`{value}` is not a boolean")),
        },
        "e" => Err(format!("an error value `{value}`")),
        _ => Err(format!("an unknown cell type `{kind}`")),
    }
}

/// The zero-based column of a cell reference such as `B3`.
fn column_index(reference: &str) -> Result<usize, String> {
    let letters: String = reference
        .chars()
        .take_while(char::is_ascii_uppercase)
        .collect();
    if letters.is_empty() || letters.len() > 3 {
        return Err(format!("cell reference `{reference}` names no column"));
    }
    Ok(letters.bytes().fold(0_usize, |index, letter| {
        index * 26 + usize::from(letter - b'A') + 1
    }) - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(id: &str, header: Option<&str>, kind: ColumnKind) -> TableFileColumn {
        TableFileColumn {
            id: id.into(),
            header: header.map(Into::into),
            kind,
            unit: None,
        }
    }

    fn programme() -> Vec<TableFileColumn> {
        vec![
            column("key_1", Some("type"), ColumnKind::TextPattern),
            column("area", Some("min_area"), ColumnKind::Number),
        ]
    }

    #[test]
    fn csv_rows_are_read_by_their_declared_headers() {
        let rows = read_table_file(
            b"\xEF\xBB\xBFtype,min_area\r\nOffice*,12.5\r\n\"Lab, \"\"wet\"\"\",\n\n",
            "rooms.csv",
            None,
            &programme(),
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0]["key_1"],
            ParameterValue::String {
                value: "Office*".into()
            }
        );
        assert_eq!(rows[0]["area"], ParameterValue::Number { value: 12.5 });
        assert_eq!(
            rows[1]["key_1"],
            ParameterValue::String {
                value: "Lab, \"wet\"".into()
            }
        );
        // An empty cell leaves its column out.
        assert!(!rows[1].contains_key("area"));
    }

    #[test]
    fn a_missing_column_an_undeclared_one_and_a_bad_cell_are_refused() {
        let read = |text: &str| read_table_file(text.as_bytes(), "rooms.csv", None, &programme());
        assert_eq!(
            read("type\nOffice\n").unwrap_err(),
            "the declared column `min_area` is missing from the header"
        );
        assert_eq!(
            read("type,min_area,note\nOffice,1,x\n").unwrap_err(),
            "the header names column `note`, which is not declared"
        );
        assert_eq!(
            read("type,min_area\nOffice,large\n").unwrap_err(),
            "row 2 column `min_area`: `large` is not a number"
        );
        assert_eq!(
            read("type,min_area\nOffice,1,2\n").unwrap_err(),
            "row 2 has 3 fields; the header has 2"
        );
        assert!(read("type,min_area\n\"Office,1\n").is_err());
        assert!(read_table_file(b"type\n", "rooms.txt", None, &programme()).is_err());
        assert!(read_table_file(b"type\n", "rooms.csv", Some("Sheet1"), &programme()).is_err());
    }

    #[test]
    fn declarations_are_checked() {
        let quantity = column("area", None, ColumnKind::Quantity);
        assert!(check_columns(std::slice::from_ref(&quantity)).is_err());
        let quantity = TableFileColumn {
            unit: Some("m2".into()),
            ..quantity
        };
        assert!(check_columns(std::slice::from_ref(&quantity)).is_ok());
        let rows = read_table_file(b"area\n3\n", "a.csv", None, &[quantity]).unwrap();
        assert_eq!(
            rows[0]["area"],
            ParameterValue::Quantity {
                value: 3.0,
                unit: "m2".into()
            }
        );
        assert!(check_columns(&[column("s", None, ColumnKind::Selector)]).is_err());
        assert!(
            check_columns(&[
                column("a", Some("x"), ColumnKind::String),
                column("b", Some("x"), ColumnKind::String)
            ])
            .is_err()
        );
    }

    #[test]
    fn paths_stay_inside_the_package() {
        for path in [
            "",
            "/etc/passwd",
            "../x.csv",
            "a/./b.csv",
            "a//b.csv",
            "C:x.csv",
            "a\\b",
        ] {
            assert!(check_path(path).is_err(), "{path}");
        }
        assert!(check_path("tables/rooms.csv").is_ok());
    }

    /// A workbook with the sheets `Notes` and `Rooms`, `Rooms` holding
    /// `cells` as its sheet data.
    fn workbook(cells: &str) -> Vec<u8> {
        use std::io::Write as _;
        let members = [
            (
                "xl/workbook.xml",
                r#"<?xml version="1.0"?><workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Notes" sheetId="1" r:id="rId1"/><sheet name="Rooms" sheetId="2" r:id="rId2"/></sheets></workbook>"#.to_owned(),
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0"?><Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Target="/xl/worksheets/sheet2.xml"/></Relationships>"#.to_owned(),
            ),
            (
                "xl/sharedStrings.xml",
                r#"<?xml version="1.0"?><sst><si><t>type</t></si><si><t>min_area</t></si><si><r><t>Office</t></r><r><t> &amp; Lab</t></r><rPh><t>x</t></rPh></si></sst>"#.to_owned(),
            ),
            ("xl/worksheets/sheet1.xml", "<worksheet><sheetData/></worksheet>".to_owned()),
            (
                "xl/worksheets/sheet2.xml",
                format!("<worksheet><sheetData>{cells}</sheetData></worksheet>"),
            ),
        ];
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, text) in members {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(text.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn a_named_sheet_of_a_workbook_is_read_by_value() {
        let header = r#"<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row>"#;
        let bytes = workbook(&format!(
            r#"{header}<row r="3"><c r="A3" t="s"><v>2</v></c><c r="B3"><v>12.5</v></c></row><row r="4"><c r="A4" t="inlineStr"><is><t>Store</t></is></c></row>"#
        ));
        let rows = read_table_file(&bytes, "rooms.xlsx", Some("Rooms"), &programme()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0]["key_1"],
            ParameterValue::String {
                value: "Office & Lab".into()
            }
        );
        assert_eq!(rows[0]["area"], ParameterValue::Number { value: 12.5 });
        assert_eq!(rows[1].len(), 1);
        assert!(
            read_table_file(&bytes, "rooms.xlsx", Some("Plans"), &programme())
                .unwrap_err()
                .contains("no sheet `Plans`")
        );
        assert!(read_table_file(&bytes, "rooms.xlsx", None, &programme()).is_err());
        let formula = workbook(&format!(
            r#"{header}<row r="2"><c r="A2" t="s"><v>2</v></c><c r="B2"><f>3*4</f><v>12</v></c></row>"#
        ));
        assert_eq!(
            read_table_file(&formula, "rooms.xlsx", Some("Rooms"), &programme()).unwrap_err(),
            "row 2 column 2: a formula; store its value instead"
        );
    }

    #[test]
    fn column_references_count_from_a() {
        assert_eq!(column_index("A1").unwrap(), 0);
        assert_eq!(column_index("Z9").unwrap(), 25);
        assert_eq!(column_index("AA10").unwrap(), 26);
        assert!(column_index("12").is_err());
    }
}
