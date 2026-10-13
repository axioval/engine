//! Catalogues of material and product data (#322), not the authoring
//! vocabulary of [`crate::catalogue`]: a ruleset declares a catalogue's
//! shape ([`CatalogueDefinition`]), the host supplies its entries per run
//! ([`SuppliedCatalogue`], [`crate::ExecutionPlan::supply_catalogue`]), and
//! an expression's `catalogue` node reads one entry's value by its key.
//!
//! A value read is the catalogue's statement, never the model's: its
//! evidence cites the catalogue, its version, the file's digest and the
//! entry ([`CATALOGUE_LOCATOR_PREFIX`]). An empty cell is a value the
//! catalogue states as absent (`null`). A key no entry has, and a catalogue
//! the host did not supply, leave the reading object not evaluated, the
//! reason naming the key; nothing is ever defaulted.
//!
//! A catalogue may hold the results of an external calculation (#323,
//! [`CalculationResults`]): a sound insulation index per wall, keyed by the
//! wall's `GlobalId`. Each entry is then read only where it states its
//! method, report, date and the model revision it was computed on, and that
//! revision is the checked model's ([`ModelRevision`], supplied per source
//! by [`crate::ExecutionPlan::supply_model_revision`]); a stale or
//! untraceable result is not evaluated, and
//! [`crate::ExecutionPlan::stale_results`] lists them for the host.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use axioval_ir::contract::{
    CalculationResults, CatalogueDefinition, CatalogueRanges, ColumnKind, ParameterValue,
    ScalarValue, TableFileColumn, TableRow,
};
use axioval_ir::{Evidence, SourceId};

use crate::expression::{Leaf, Value, parse_unit};

/// The locator prefix of a catalogue value's evidence:
/// `axioval:catalogue;id=<id>;version=<version>;sha256=<digest>#entry=<key>`
/// (`version` omitted when the host stated none).
pub const CATALOGUE_LOCATOR_PREFIX: &str = "axioval:catalogue";

/// The entries of a catalogue as the host read them at check time: their
/// origin, the SHA-256 of the bytes they were read from, the catalogue's
/// version as the host states it, and one row per entry.
#[derive(Clone, Debug, PartialEq)]
pub struct SuppliedCatalogue {
    origin: String,
    sha256: String,
    version: Option<String>,
    rows: Vec<TableRow>,
}

impl SuppliedCatalogue {
    /// Entries the host already read: `origin` names where they came from
    /// (such as a file name), `sha256` is the lowercase hex SHA-256 of the
    /// bytes they were read from and `version` the catalogue's edition,
    /// each recorded in every value's evidence.
    #[must_use]
    pub fn new(
        origin: impl Into<String>,
        sha256: impl Into<String>,
        version: Option<String>,
        rows: Vec<TableRow>,
    ) -> Self {
        Self {
            origin: origin.into(),
            sha256: sha256.into(),
            version,
            rows,
        }
    }

    /// Reads the entries of the catalogue `definition` from `bytes`, the
    /// file `name` (a `.csv` file, or the sheet `sheet` of an `.xlsx`
    /// workbook), by the columns it declares, exactly as a table file is
    /// read ([`read_table_file`](crate::read_table_file)): an empty cell
    /// leaves its column out of the entry.
    ///
    /// # Errors
    ///
    /// Why the file or its columns were refused.
    pub fn read(
        definition: &CatalogueDefinition,
        bytes: &[u8],
        name: &str,
        sheet: Option<&str>,
        version: Option<&str>,
    ) -> Result<Self, String> {
        let rows = crate::table_files::read_table_file(bytes, name, sheet, &definition.columns)?;
        let origin = match sheet {
            Some(sheet) => format!("{name}#{sheet}"),
            None => name.to_owned(),
        };
        Ok(Self::new(
            origin,
            crate::table_files::sha256_hex(bytes),
            version.map(str::to_owned),
            rows,
        ))
    }

    /// Where the entries came from.
    #[must_use]
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// The SHA-256 of the bytes the entries were read from.
    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// The catalogue's version, as the host states it.
    #[must_use]
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// The entries, one row each, in the file's order.
    #[must_use]
    pub fn rows(&self) -> &[TableRow] {
        &self.rows
    }
}

/// The catalogue `id` a ruleset declares, checked: an id usable in a
/// locator, a declared `string` key column, columns a file can hold, each
/// of a value kind, and every quantity's unit one the engine reads.
pub(crate) fn check_definition(id: &str, definition: &CatalogueDefinition) -> Result<(), String> {
    if id != definition.id {
        return Err(format!("is declared under the key `{id}`, not its id"));
    }
    if definition.id.trim().is_empty()
        || definition
            .id
            .chars()
            .any(|c| matches!(c, ':' | ';' | '|' | '/' | '#' | '=') || c.is_whitespace())
    {
        return Err(
            "its id must not be blank or hold `:`, `;`, `|`, `/`, `#`, `=` or whitespace".into(),
        );
    }
    crate::table_files::check_columns(&definition.columns)
        .map_err(|detail| format!("its columns: {detail}"))?;
    let key = definition
        .columns
        .iter()
        .find(|column| column.id == definition.key)
        .ok_or_else(|| format!("its key column `{}` is not declared", definition.key))?;
    if key.kind != ColumnKind::String {
        return Err(format!(
            "its key column `{}` is declared {}, not string",
            key.id,
            key.kind.as_str()
        ));
    }
    for column in &definition.columns {
        if matches!(column.kind, ColumnKind::Reference | ColumnKind::TextPattern) {
            return Err(format!(
                "its column `{}` is declared {}, which states no value",
                column.id,
                column.kind.as_str()
            ));
        }
        if let Some(unit) = &column.unit {
            parse_unit(unit).map_err(|why| format!("its column `{}`: {why}", column.id))?;
        }
    }
    if let Some(results) = &definition.results {
        check_results(definition, results)?;
    }
    if let Some(ranges) = &definition.ranges {
        check_ranges(definition, ranges)?;
    }
    Ok(())
}

/// The range columns of a catalogue stated per range, checked: two
/// distinct declared columns other than the key, both `number` or both
/// `quantity` of one unit.
fn check_ranges(definition: &CatalogueDefinition, ranges: &CatalogueRanges) -> Result<(), String> {
    let column = |role: &str, id: &str| {
        let column = definition
            .columns
            .iter()
            .find(|column| column.id == id)
            .ok_or_else(|| format!("its ranges' {role} column `{id}` is not declared"))?;
        if id == definition.key {
            return Err(format!(
                "its ranges' {role} column `{id}` is its key, which names the entries"
            ));
        }
        if !matches!(column.kind, ColumnKind::Number | ColumnKind::Quantity) {
            return Err(format!(
                "its ranges' {role} column `{id}` is declared {}, not number or quantity",
                column.kind.as_str()
            ));
        }
        Ok(column)
    };
    let (from, to) = (column("from", &ranges.from)?, column("to", &ranges.to)?);
    if from.id == to.id {
        return Err(format!(
            "its ranges name the column `{}` as both from and to",
            from.id
        ));
    }
    let unit = |column: &TableFileColumn| column.unit.as_deref().map(parse_unit);
    let same = match (unit(from), unit(to)) {
        (None, None) => from.kind == to.kind,
        (Some(Ok((_, from_unit))), Some(Ok((_, to_unit)))) => from_unit == to_unit,
        _ => false,
    };
    if !same {
        return Err(format!(
            "its ranges' columns `{}` and `{}` state positions of different kinds or units",
            from.id, to.id
        ));
    }
    Ok(())
}

/// One range of a catalogue stated per range: its ends in coherent units
/// (`[from, to)`), as stated for the locator, and its entry's row.
#[derive(Clone, Debug, PartialEq)]
struct Range {
    from: f64,
    to: f64,
    stated: String,
    row: usize,
}

/// A range column's cell in coherent units, and as stated.
fn position(cell: Option<&ParameterValue>) -> Option<(f64, String)> {
    let cell = cell?;
    let stated = match cell {
        ParameterValue::Number { value } => value.to_string(),
        ParameterValue::Quantity { value, unit } => format!("{value} {unit}"),
        _ => return None,
    };
    match ScalarValue::try_from(cell.clone())
        .ok()
        .and_then(|scalar| Value::from_literal(&scalar).ok())?
    {
        Value::Number { value, .. } if value.is_point() => Some((value.lower, stated)),
        _ => None,
    }
}

/// The unit of the positions a catalogue's ranges state: its `from`
/// column's, plain for a number.
fn range_unit(definition: &CatalogueDefinition) -> Result<crate::expression::Unit, String> {
    definition
        .ranges
        .as_ref()
        .and_then(|ranges| {
            definition
                .columns
                .iter()
                .find(|column| column.id == ranges.from)
        })
        .and_then(|column| column.unit.as_deref())
        .map_or(Ok(crate::expression::Unit::NONE), |unit| {
            parse_unit(unit).map(|(_, unit)| unit)
        })
}

/// The `column` value of an entry's `row` of `catalogue`: `null` for an
/// empty cell.
fn cell_value(catalogue: &str, column: &str, row: &TableRow) -> Result<Value, String> {
    match row.get(column) {
        None => Ok(Value::Null),
        Some(cell) => ScalarValue::try_from(cell.clone())
            .map_err(|_| format!("catalogue `{catalogue}` column `{column}` states no value"))
            .and_then(|scalar| Value::from_literal(&scalar)),
    }
}

/// The ranges of each key of a catalogue stated per range, sorted by
/// start: every entry stating both ends, its start below its end, and no
/// two ranges of one key overlapping.
fn index_ranges(
    ranges: &CatalogueRanges,
    key: &str,
    rows: &[TableRow],
) -> Result<BTreeMap<String, Vec<Range>>, String> {
    let mut keyed: BTreeMap<String, Vec<Range>> = BTreeMap::new();
    for (row, cells) in rows.iter().enumerate() {
        let entry = row + 1;
        let Some(ParameterValue::String { value: name }) = cells.get(key) else {
            continue;
        };
        let (Some(from), Some(to)) = (
            position(cells.get(&ranges.from)),
            position(cells.get(&ranges.to)),
        ) else {
            return Err(format!(
                "entry {entry} states no range (`{}` and `{}`)",
                ranges.from, ranges.to
            ));
        };
        if from.0 >= to.0 {
            return Err(format!(
                "entry {entry}'s range starts at {} and ends at {}, not after it",
                from.1, to.1
            ));
        }
        keyed.entry(name.clone()).or_default().push(Range {
            from: from.0,
            to: to.0,
            stated: format!("{}..{}", from.1, to.1),
            row,
        });
    }
    for (name, ranges) in &mut keyed {
        ranges.sort_by(|a, b| a.from.total_cmp(&b.from));
        if let Some(pair) = ranges.windows(2).find(|pair| pair[1].from < pair[0].to) {
            return Err(format!(
                "entries {} and {} of `{name}` state overlapping ranges {} and {}",
                pair[0].row + 1,
                pair[1].row + 1,
                pair[0].stated,
                pair[1].stated
            ));
        }
    }
    Ok(keyed)
}

/// The provenance columns of a results catalogue, checked: each a declared
/// column other than the key, of its kind, and no column named twice.
fn check_results(
    definition: &CatalogueDefinition,
    results: &CalculationResults,
) -> Result<(), String> {
    let named = [
        ("method", &results.method),
        ("report", &results.report),
        ("date", &results.date),
        ("revision", &results.revision),
    ];
    for (index, (role, id)) in named.iter().enumerate() {
        let column = definition
            .columns
            .iter()
            .find(|column| column.id == **id)
            .ok_or_else(|| format!("its results' {role} column `{id}` is not declared"))?;
        if **id == definition.key {
            return Err(format!(
                "its results' {role} column `{id}` is its key, which names the entries"
            ));
        }
        let (fits, wanted) = if *role == "date" {
            (
                matches!(column.kind, ColumnKind::Date | ColumnKind::DateTime),
                "date or dateTime",
            )
        } else {
            (column.kind == ColumnKind::String, "string")
        };
        if !fits {
            return Err(format!(
                "its results' {role} column `{id}` is declared {}, not {wanted}",
                column.kind.as_str()
            ));
        }
        if let Some((other, _)) = named[..index].iter().find(|(_, other)| *other == *id) {
            return Err(format!(
                "its results name the column `{id}` as both {other} and {role}"
            ));
        }
    }
    Ok(())
}

/// The revision of a checked model, as a host identifies it (#323): the
/// SHA-256 of the model file's bytes, and optionally a revision the host
/// states for it (a document version of a common data environment). A
/// result's stated revision names it when it is `sha256:<hex>` of that
/// digest (hex in any case) or the stated revision exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelRevision {
    sha256: String,
    stated: Option<String>,
}

impl ModelRevision {
    /// The revision of a model file whose bytes have the hex SHA-256
    /// `sha256`, with the revision `stated` for it, if any.
    ///
    /// # Errors
    ///
    /// A digest that is not 64 hex digits, or a blank stated revision.
    pub fn new(sha256: impl Into<String>, stated: Option<String>) -> Result<Self, String> {
        let sha256 = sha256.into().to_ascii_lowercase();
        if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("`{sha256}` is not a SHA-256 digest in hex"));
        }
        if stated
            .as_ref()
            .is_some_and(|stated| stated.trim().is_empty())
        {
            return Err("the stated revision is blank".into());
        }
        Ok(Self {
            sha256,
            stated: stated.map(|stated| stated.trim().to_owned()),
        })
    }

    /// The revision of the model file `bytes`, with the revision `stated`
    /// for it, if any.
    ///
    /// # Errors
    ///
    /// A blank stated revision.
    pub fn of_bytes(bytes: &[u8], stated: Option<String>) -> Result<Self, String> {
        Self::new(crate::table_files::sha256_hex(bytes), stated)
    }

    /// The lowercase hex SHA-256 of the model file.
    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// The revision the host states for the model, if any.
    #[must_use]
    pub fn stated(&self) -> Option<&str> {
        self.stated.as_deref()
    }

    /// Whether a result's stated `revision` names this one.
    #[must_use]
    pub fn identifies(&self, revision: &str) -> bool {
        let revision = revision.trim();
        revision
            .strip_prefix("sha256:")
            .is_some_and(|digest| digest.eq_ignore_ascii_case(&self.sha256))
            || self.stated.as_deref() == Some(revision)
    }
}

impl std::fmt::Display for ModelRevision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sha256:{}", self.sha256)?;
        if let Some(stated) = &self.stated {
            write!(f, " (revision `{stated}`)")?;
        }
        Ok(())
    }
}

/// A result of an external calculation that a run will not read (#323):
/// computed on another revision than every checked model's, or stating no
/// provenance to trace it by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaleResult {
    /// The results catalogue.
    pub catalogue: String,
    /// The entry's key.
    pub key: String,
    /// The revision the entry states it was computed on; `None` where it
    /// states none.
    pub revision: Option<String>,
    /// Whether it is stale (`true`: computed on another revision) or
    /// untraceable (`false`: a provenance cell is empty).
    pub stale: bool,
    /// Why, in plain words.
    pub why: String,
}

/// The provenance of one result entry, as its cells state it.
struct Provenance {
    method: String,
    report: String,
    date: String,
    revision: String,
}

/// The provenance `results` names in `row`, or why it cannot be traced.
fn provenance(results: &CalculationResults, row: &TableRow) -> Result<Provenance, String> {
    let cell = |role: &str, id: &str| -> Result<String, String> {
        match row.get(id) {
            Some(ParameterValue::String { value }) if !value.trim().is_empty() => {
                Ok(value.trim().to_owned())
            }
            Some(ParameterValue::Date { value }) => Ok(value.to_string()),
            Some(ParameterValue::DateTime { value }) => Ok(value.to_string()),
            _ => Err(format!("states no {role} (`{id}`)")),
        }
    };
    Ok(Provenance {
        method: cell("method", &results.method)?,
        report: cell("report", &results.report)?,
        date: cell("date", &results.date)?,
        revision: cell("model revision", &results.revision)?,
    })
}

/// `text` safe in a locator's field: `%`, `;`, `#`, `=` and control
/// characters percent-encoded.
fn locator_text(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '%' | ';' | '#' | '=') || c.is_control() {
            let mut bytes = [0; 4];
            for byte in c.encode_utf8(&mut bytes).bytes() {
                let _ = write!(encoded, "%{byte:02X}");
            }
        } else {
            encoded.push(c);
        }
    }
    encoded
}

/// Checks the supplied entries against their declaration: every cell of a
/// declared column and of its kind (a quantity in a unit of the declared
/// one's dimension), every entry naming a non-blank key, no key twice. The
/// entries by key, each its row's index.
fn index(
    definition: &CatalogueDefinition,
    rows: &[TableRow],
) -> Result<BTreeMap<String, usize>, String> {
    let mut entries = BTreeMap::new();
    for (index, row) in rows.iter().enumerate() {
        let entry = index + 1;
        for (id, cell) in row {
            let column = definition
                .columns
                .iter()
                .find(|column| column.id == *id)
                .ok_or_else(|| format!("entry {entry} fills the undeclared column `{id}`"))?;
            cell_fits(column, cell).map_err(|why| format!("entry {entry} column `{id}`: {why}"))?;
        }
        let key = match row.get(&definition.key) {
            Some(ParameterValue::String { value }) if !value.trim().is_empty() => value,
            _ => return Err(format!("entry {entry} names no key")),
        };
        // Stated per range, a key names an entry per range.
        if definition.ranges.is_some() {
            entries.entry(key.clone()).or_insert(index);
            continue;
        }
        if let Some(earlier) = entries.insert(key.clone(), index) {
            return Err(format!(
                "entries {} and {entry} both name the key `{key}`",
                earlier + 1
            ));
        }
    }
    Ok(entries)
}

/// Whether `cell` is a value of `column`'s kind.
fn cell_fits(column: &TableFileColumn, cell: &ParameterValue) -> Result<(), String> {
    let fits = match (column.kind, cell) {
        (ColumnKind::String, ParameterValue::String { .. })
        | (ColumnKind::Number, ParameterValue::Number { .. })
        | (ColumnKind::Integer, ParameterValue::Integer { .. })
        | (ColumnKind::Boolean, ParameterValue::Boolean { .. })
        | (ColumnKind::Date, ParameterValue::Date { .. })
        | (ColumnKind::DateTime, ParameterValue::DateTime { .. }) => true,
        (ColumnKind::Quantity, ParameterValue::Quantity { unit, .. }) => {
            let declared = column.unit.as_deref().map(parse_unit);
            match (declared, parse_unit(unit)) {
                (Some(Ok((_, declared))), Ok((_, stated))) => declared == stated,
                _ => false,
            }
        }
        _ => false,
    };
    if fits {
        Ok(())
    } else {
        Err(format!("a {} cell is needed", column.kind.as_str()))
    }
}

/// One supplied catalogue, checked and indexed by key.
#[derive(Clone, Debug, PartialEq)]
struct Indexed {
    supplied: SuppliedCatalogue,
    entries: BTreeMap<String, usize>,
    /// Stated per range: each key's ranges, sorted.
    ranges: BTreeMap<String, Vec<Range>>,
}

/// The catalogues of a run: every one the plan declares, with the entries
/// the host supplied for it. Installed in the run's services (as
/// `Arc<Catalogues>`), where every expression reads them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalogues {
    declared: BTreeMap<String, CatalogueDefinition>,
    supplied: BTreeMap<String, Indexed>,
    revisions: BTreeMap<SourceId, ModelRevision>,
}

impl Catalogues {
    /// The catalogues `declared`, none supplied yet.
    pub(crate) fn declared(declared: &[CatalogueDefinition]) -> Self {
        Self {
            declared: declared
                .iter()
                .map(|definition| (definition.id.clone(), definition.clone()))
                .collect(),
            supplied: BTreeMap::new(),
            revisions: BTreeMap::new(),
        }
    }

    /// The definitions, by id.
    pub(crate) fn definitions(&self) -> impl Iterator<Item = &CatalogueDefinition> {
        self.declared.values()
    }

    /// The entries supplied for `id`, if any.
    pub(crate) fn supplied(&self, id: &str) -> Option<&SuppliedCatalogue> {
        self.supplied.get(id).map(|indexed| &indexed.supplied)
    }

    /// Supplies the entries of `id`.
    pub(crate) fn supply(&mut self, id: &str, supplied: SuppliedCatalogue) -> Result<(), String> {
        let definition = self.declared.get(id).ok_or_else(|| {
            "no ruleset declares it, so its entries cannot be supplied".to_owned()
        })?;
        if self.supplied.contains_key(id) {
            return Err("its entries are supplied twice".into());
        }
        if supplied
            .version
            .as_ref()
            .is_some_and(|version| version.trim().is_empty())
        {
            return Err("its version is blank".into());
        }
        let entries = index(definition, &supplied.rows)
            .map_err(|detail| format!("the entries of `{}`: {detail}", supplied.origin))?;
        let ranges = match &definition.ranges {
            None => BTreeMap::new(),
            Some(ranges) => index_ranges(ranges, &definition.key, &supplied.rows)
                .map_err(|detail| format!("the entries of `{}`: {detail}", supplied.origin))?,
        };
        self.supplied.insert(
            id.to_owned(),
            Indexed {
                supplied,
                entries,
                ranges,
            },
        );
        Ok(())
    }

    /// States the revision of the checked model `source`.
    pub(crate) fn supply_revision(
        &mut self,
        source: SourceId,
        revision: ModelRevision,
    ) -> Result<(), String> {
        if self.revisions.contains_key(&source) {
            return Err(format!("the revision of {source} is supplied twice"));
        }
        self.revisions.insert(source, revision);
        Ok(())
    }

    /// The revision supplied for the checked model `source`, if any.
    pub(crate) fn revision(&self, source: &SourceId) -> Option<&ModelRevision> {
        self.revisions.get(source)
    }

    /// Whether any declared catalogue holds calculation results.
    pub(crate) fn has_results(&self) -> bool {
        self.declared
            .values()
            .any(|definition| definition.results.is_some())
    }

    /// Every entry of a supplied results catalogue that no run reads:
    /// computed on a revision no checked model has, or stating no
    /// provenance, by catalogue and then the file's order.
    pub(crate) fn stale_results(&self) -> Vec<StaleResult> {
        let mut stale = Vec::new();
        for (id, Indexed { supplied, .. }) in &self.supplied {
            let Some(results) = self
                .declared
                .get(id)
                .and_then(|definition| definition.results.as_ref())
            else {
                continue;
            };
            let key_column = &self.declared[id].key;
            for row in &supplied.rows {
                let Some(ParameterValue::String { value: key }) = row.get(key_column) else {
                    continue;
                };
                match provenance(results, row) {
                    Err(why) => stale.push(StaleResult {
                        catalogue: id.clone(),
                        key: key.clone(),
                        revision: None,
                        stale: false,
                        why: format!("result `{key}` of catalogue `{id}` {why}"),
                    }),
                    Ok(traced) => {
                        if self
                            .revisions
                            .values()
                            .any(|revision| revision.identifies(&traced.revision))
                        {
                            continue;
                        }
                        let checked = if self.revisions.is_empty() {
                            "no checked model's revision is stated".to_owned()
                        } else {
                            format!(
                                "the checked model{} {}",
                                if self.revisions.len() == 1 {
                                    " is"
                                } else {
                                    "s are"
                                },
                                self.revisions
                                    .values()
                                    .map(ToString::to_string)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        };
                        stale.push(StaleResult {
                            catalogue: id.clone(),
                            key: key.clone(),
                            why: format!(
                                "result `{key}` of catalogue `{id}` was computed on model \
                                 revision `{}`, and {checked}",
                                traced.revision
                            ),
                            revision: Some(traced.revision),
                            stale: true,
                        });
                    }
                }
            }
        }
        stale
    }

    /// The `column` value of the entry of `catalogue` whose key is `key`,
    /// as the catalogue states it, for an object of `source`: the value
    /// with evidence citing the catalogue, its version and the entry,
    /// `null` for an empty cell, and unreadable, naming the key, where no
    /// entry has it or no entries were supplied.
    #[must_use]
    pub fn read(&self, catalogue: &str, key: &str, column: &str, source: &SourceId) -> Leaf {
        let Some(definition) = self.declared.get(catalogue) else {
            return Leaf::unreadable(format!("no catalogue `{catalogue}` is declared"));
        };
        let Some(Indexed {
            supplied, entries, ..
        }) = self.supplied.get(catalogue)
        else {
            return Leaf::unreadable(format!(
                "catalogue `{catalogue}` takes its entries from the host at check time, and \
                 none were supplied, so its entry `{key}` cannot be read"
            ));
        };
        if definition.ranges.is_some() {
            return Leaf::unreadable(format!(
                "catalogue `{catalogue}` states its entries per range, so it is read at a position"
            ));
        }
        let edition = supplied
            .version
            .as_ref()
            .map_or_else(String::new, |version| format!(" (version {version})"));
        let Some(&index) = entries.get(key) else {
            return Leaf::unreadable(format!(
                "catalogue `{catalogue}`{edition} has no entry whose `{}` is `{key}`",
                definition.key
            ));
        };
        let version = supplied
            .version
            .as_ref()
            .map_or_else(String::new, |version| format!(";version={version}"));
        let traced = match &definition.results {
            None => String::new(),
            Some(results) => {
                match self.traced(catalogue, key, results, &supplied.rows[index], source) {
                    Ok(traced) => traced,
                    Err(why) => return Leaf::unreadable(why),
                }
            }
        };
        let evidence = Evidence::exact(
            source.clone(),
            format!(
                "{CATALOGUE_LOCATOR_PREFIX};id={catalogue}{version};sha256={}#entry={key}{traced}",
                supplied.sha256
            ),
        );
        Leaf {
            value: cell_value(catalogue, column, &supplied.rows[index]),
            evidence: vec![evidence],
        }
    }
}

impl Catalogues {
    /// The `column` value of the entry of `catalogue` whose key is `key`
    /// and whose range `[from, to)` holds the position `at` (#324), for an
    /// object of `source`, as [`Catalogues::read`] reads one entry, its
    /// evidence citing the range (`#entry=<key>;range=<from>..<to>`). A
    /// position whose interval reaches several ranges reads their value
    /// where they state one alike; one reaching ranges stating different
    /// values, or reaching no range or a gap between ranges, is
    /// unreadable, naming the position, never a value picked.
    #[must_use]
    pub fn read_at(
        &self,
        catalogue: &str,
        key: &str,
        at: &Value,
        column: &str,
        source: &SourceId,
    ) -> Leaf {
        let Some(definition) = self.declared.get(catalogue) else {
            return Leaf::unreadable(format!("no catalogue `{catalogue}` is declared"));
        };
        if definition.ranges.is_none() {
            return Leaf::unreadable(format!(
                "catalogue `{catalogue}` states one entry per key, not ranges, so it is not read \
                 at a position"
            ));
        }
        let Some(Indexed {
            supplied, ranges, ..
        }) = self.supplied.get(catalogue)
        else {
            return Leaf::unreadable(format!(
                "catalogue `{catalogue}` takes its entries from the host at check time, and \
                 none were supplied, so `{key}` at {at} cannot be read"
            ));
        };
        let edition = supplied
            .version
            .as_ref()
            .map_or_else(String::new, |version| format!(" (version {version})"));
        let Some(stated) = ranges.get(key) else {
            return Leaf::unreadable(format!(
                "catalogue `{catalogue}`{edition} has no entry whose `{}` is `{key}`",
                definition.key
            ));
        };
        let Value::Number { value: held, unit } = at else {
            return Leaf::unreadable(format!(
                "catalogue `{catalogue}` is read at a number, not {}",
                at.kind()
            ));
        };
        if range_unit(definition).as_ref() != Ok(unit) {
            return Leaf::unreadable(format!(
                "catalogue `{catalogue}` states its ranges in another unit than {at}"
            ));
        }
        let reached: Vec<&Range> = stated
            .iter()
            .filter(|range| range.from <= held.upper && held.lower < range.to)
            .collect();
        let covered = reached
            .first()
            .is_some_and(|first| first.from <= held.lower)
            && reached.last().is_some_and(|last| held.upper < last.to)
            && reached.windows(2).all(|pair| pair[0].to >= pair[1].from);
        if !covered {
            let listed: Vec<&str> = stated.iter().map(|range| range.stated.as_str()).collect();
            return Leaf::unreadable(format!(
                "catalogue `{catalogue}`{edition} states no range of `{key}` holding {at} \
                 (its ranges: {})",
                listed.join(", ")
            ));
        }
        let version = supplied
            .version
            .as_ref()
            .map_or_else(String::new, |version| format!(";version={version}"));
        let mut value: Option<Result<Value, String>> = None;
        let mut evidence = Vec::with_capacity(reached.len());
        for range in &reached {
            let row = &supplied.rows[range.row];
            let traced = match &definition.results {
                None => String::new(),
                Some(results) => match self.traced(catalogue, key, results, row, source) {
                    Ok(traced) => traced,
                    Err(why) => return Leaf::unreadable(why),
                },
            };
            evidence.push(Evidence::exact(
                source.clone(),
                format!(
                    "{CATALOGUE_LOCATOR_PREFIX};id={catalogue}{version};sha256={}#entry={key};\
                     range={}{traced}",
                    supplied.sha256,
                    locator_text(&range.stated)
                ),
            ));
            let read = cell_value(catalogue, column, row);
            match &value {
                None => value = Some(read),
                Some(earlier) if *earlier == read => {}
                Some(_) => {
                    let listed: Vec<&str> =
                        reached.iter().map(|range| range.stated.as_str()).collect();
                    return Leaf::unreadable(format!(
                        "{at} may lie in any of the ranges {} of `{key}` in catalogue \
                         `{catalogue}`{edition}, which state different `{column}`, so which \
                         one holds it is undecided",
                        listed.join(", ")
                    ));
                }
            }
        }
        Leaf {
            value: value.unwrap_or(Ok(Value::Null)),
            evidence,
        }
    }

    /// The locator fields tracing the result `key` of `catalogue` read for
    /// an object of `source` (`;method=…;report=…;date=…;revision=…`), or
    /// why it is not read: untraceable, the checked model's revision
    /// unknown, or stale.
    fn traced(
        &self,
        catalogue: &str,
        key: &str,
        results: &CalculationResults,
        row: &TableRow,
        source: &SourceId,
    ) -> Result<String, String> {
        let traced = provenance(results, row).map_err(|why| {
            format!(
                "result `{key}` of catalogue `{catalogue}` {why}, so it cannot be traced to \
                 the calculation that computed it"
            )
        })?;
        let Some(checked) = self.revisions.get(source) else {
            return Err(format!(
                "result `{key}` of catalogue `{catalogue}` was computed on model revision `{}`, \
                 and the revision of the checked model {source} is not stated, so the result \
                 cannot be matched to it",
                traced.revision
            ));
        };
        if !checked.identifies(&traced.revision) {
            return Err(format!(
                "result `{key}` of catalogue `{catalogue}` is stale: it was computed on model \
                 revision `{}`, and the checked model {source} is {checked}",
                traced.revision
            ));
        }
        Ok(format!(
            ";method={};report={};date={};revision={}",
            locator_text(&traced.method),
            locator_text(&traced.report),
            locator_text(&traced.date),
            locator_text(&traced.revision)
        ))
    }
}

/// The `column` value of the entry `key` of `catalogue` whose range holds
/// `at`, in the run whose services these are, for an object of `source`
/// ([`Catalogues::read_at`]); unreadable outside a run.
#[must_use]
pub fn catalogue_value_at(
    services: &crate::ServiceRegistry,
    (catalogue, key, at): (&str, &str, &Value),
    column: &str,
    source: &SourceId,
) -> Leaf {
    match services.get::<Arc<Catalogues>>() {
        Some(catalogues) => catalogues.read_at(catalogue, key, at, column, source),
        None => Leaf::unreadable(format!(
            "catalogue `{catalogue}` cannot be read outside a run"
        )),
    }
}

/// The `column` value of the entry `key` of `catalogue` in the run whose
/// services these are, for an object of `source`, as an expression reads
/// it ([`Catalogues::read`]); unreadable outside a run.
#[must_use]
pub fn catalogue_value(
    services: &crate::ServiceRegistry,
    catalogue: &str,
    key: &str,
    column: &str,
    source: &SourceId,
) -> Leaf {
    match services.get::<Arc<Catalogues>>() {
        Some(catalogues) => catalogues.read(catalogue, key, column, source),
        None => Leaf::unreadable(format!(
            "catalogue `{catalogue}` cannot be read outside a run"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition() -> CatalogueDefinition {
        serde_json::from_value(serde_json::json!({
            "id": "materials",
            "name": {"default": "Materials", "translations": {}},
            "key": "name",
            "columns": [
                {"id": "name", "kind": "string"},
                {"id": "lambda", "header": "λ", "kind": "quantity", "unit": "W/m·K"},
                {"id": "density", "kind": "quantity", "unit": "kg/m³"},
            ],
        }))
        .unwrap()
    }

    fn source() -> SourceId {
        SourceId::new("ifc-step", "model.ifc").unwrap()
    }

    fn catalogue(csv: &str) -> Catalogues {
        let definition = definition();
        let mut catalogues = Catalogues::declared(std::slice::from_ref(&definition));
        let supplied =
            SuppliedCatalogue::read(&definition, csv.as_bytes(), "m.csv", None, Some("2026-1"))
                .unwrap();
        catalogues.supply("materials", supplied).unwrap();
        catalogues
    }

    #[test]
    fn an_entry_is_read_by_its_key_with_its_provenance() {
        let catalogues = catalogue(
            "name,λ,density\nex-clay-brick,0.81,1800\nex-foam-board,0.035,\nex-wood-fibre,,160\n",
        );
        let leaf = catalogues.read("materials", "ex-foam-board", "lambda", &source());
        let Ok(Value::Number { value, unit }) = &leaf.value else {
            panic!("{leaf:?}");
        };
        assert!((value.lower - 0.035).abs() < 1e-12 && value.is_point());
        assert_eq!(unit.to_string(), parse_unit("W/m·K").unwrap().1.to_string());
        let digest = crate::table_files::sha256_hex(
            b"name,\xce\xbb,density\nex-clay-brick,0.81,1800\nex-foam-board,0.035,\nex-wood-fibre,,160\n",
        );
        assert_eq!(
            leaf.evidence,
            [Evidence::exact(
                source(),
                format!(
                    "axioval:catalogue;id=materials;version=2026-1;sha256={digest}\
                     #entry=ex-foam-board"
                )
            )]
        );
        // An empty cell is a value the catalogue states as absent.
        let empty = catalogues.read("materials", "ex-wood-fibre", "lambda", &source());
        assert_eq!(empty.value, Ok(Value::Null));
        assert_eq!(empty.evidence.len(), 1);
    }

    #[test]
    fn a_key_without_an_entry_and_an_unsupplied_catalogue_are_unreadable() {
        let catalogues = catalogue("name,λ,density\nex-clay-brick,0.81,1800\n");
        let missing = catalogues.read("materials", "ex-mystery-board", "lambda", &source());
        assert_eq!(
            missing.value,
            Err(
                "catalogue `materials` (version 2026-1) has no entry whose `name` is \
                 `ex-mystery-board`"
                    .into()
            )
        );
        assert!(missing.evidence.is_empty());
        // Keys match exactly.
        assert!(
            catalogues
                .read("materials", "EX-CLAY-BRICK", "lambda", &source())
                .value
                .is_err()
        );
        let unsupplied = Catalogues::declared(&[definition()]);
        let leaf = unsupplied.read("materials", "ex-clay-brick", "lambda", &source());
        assert!(
            leaf.value.as_ref().is_err_and(
                |why| why.contains("none were supplied") && why.contains("`ex-clay-brick`")
            ),
            "{leaf:?}"
        );
    }

    #[test]
    fn supplied_entries_are_checked_against_their_declaration() {
        let definition = definition();
        let supply = |rows: Vec<TableRow>| {
            let mut catalogues = Catalogues::declared(std::slice::from_ref(&definition));
            catalogues.supply(
                "materials",
                SuppliedCatalogue::new("m.csv", "00", None, rows),
            )
        };
        let text = |value: &str| ParameterValue::String {
            value: value.into(),
        };
        let row = |cells: Vec<(&str, ParameterValue)>| -> TableRow {
            cells
                .into_iter()
                .map(|(id, cell)| (id.to_owned(), cell))
                .collect()
        };
        assert!(
            supply(vec![
                row(vec![("name", text("a"))]),
                row(vec![("name", text("a"))])
            ])
            .is_err_and(|why| why.contains("entries 1 and 2 both name the key `a`"))
        );
        assert!(
            supply(vec![row(vec![("lambda", text("x"))])])
                .is_err_and(|why| why.contains("a quantity cell is needed"))
        );
        assert!(
            supply(vec![row(vec![(
                "density",
                ParameterValue::Number { value: 1.0 }
            )])])
            .is_err_and(|why| why.contains("a quantity cell is needed"))
        );
        assert!(
            supply(vec![row(vec![("name", text(" "))])])
                .is_err_and(|why| why.contains("names no key"))
        );
        // A quantity in another unit of the declared dimension is read as stated.
        assert!(
            supply(vec![row(vec![
                ("name", text("a")),
                (
                    "density",
                    ParameterValue::Quantity {
                        value: 1.8,
                        unit: "g/cm³".into()
                    }
                ),
            ])])
            .is_ok()
        );
        let mut catalogues = Catalogues::declared(std::slice::from_ref(&definition));
        assert!(
            catalogues
                .supply(
                    "other",
                    SuppliedCatalogue::new("m.csv", "00", None, Vec::new())
                )
                .is_err()
        );
    }

    #[test]
    fn declarations_are_checked() {
        let checked = |edit: &dyn Fn(&mut CatalogueDefinition)| {
            let mut definition = definition();
            edit(&mut definition);
            check_definition(&definition.id.clone(), &definition)
        };
        assert!(check_definition("other", &definition()).is_err());
        assert_eq!(checked(&|_| {}), Ok(()));
        assert!(checked(&|definition| definition.key = "colour".into()).is_err());
        assert!(checked(&|definition| definition.key = "lambda".into()).is_err());
        assert!(checked(&|definition| definition.id = "mat:erials".into()).is_err());
        assert!(
            checked(&|definition| definition.columns[1].unit = Some("furlong".into())).is_err()
        );
        assert!(checked(&|definition| definition.columns[1].kind = ColumnKind::Reference).is_err());
    }

    /// A results catalogue of sound insulation indices per wall, keyed by
    /// the wall's `GlobalId`.
    fn results() -> CatalogueDefinition {
        serde_json::from_value(serde_json::json!({
            "id": "sound",
            "name": {"default": "Sound insulation", "translations": {}},
            "key": "wall",
            "columns": [
                {"id": "wall", "kind": "string"},
                {"id": "rw", "header": "R'w", "kind": "number"},
                {"id": "method", "kind": "string"},
                {"id": "report", "kind": "string"},
                {"id": "date", "kind": "date"},
                {"id": "revision", "kind": "string"},
            ],
            "results": {"method": "method", "report": "report", "date": "date",
                        "revision": "revision"},
        }))
        .unwrap()
    }

    const DIGEST: &str = "ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12";

    fn sound(csv: &str, revision: Option<ModelRevision>) -> Catalogues {
        let definition = results();
        check_definition("sound", &definition).unwrap();
        let mut catalogues = Catalogues::declared(std::slice::from_ref(&definition));
        let supplied =
            SuppliedCatalogue::read(&definition, csv.as_bytes(), "rw.csv", None, None).unwrap();
        catalogues.supply("sound", supplied).unwrap();
        if let Some(revision) = revision {
            catalogues.supply_revision(source(), revision).unwrap();
        }
        catalogues
    }

    #[test]
    fn a_result_is_read_on_the_revision_it_was_computed_on_and_traced() {
        let csv = format!(
            "wall,R'w,method,report,date,revision\n\
             W1,53,ex-method 2018,\"EX-R 7; p. 3\",2026-09-01,sha256:{}\n\
             W2,49,ex-method 2018,EX-R 7,2026-09-01,rev-B\n",
            DIGEST.to_uppercase()
        );
        let catalogues = sound(&csv, Some(ModelRevision::new(DIGEST, None).unwrap()));
        let leaf = catalogues.read("sound", "W1", "rw", &source());
        assert!(matches!(leaf.value, Ok(Value::Number { .. })), "{leaf:?}");
        assert!(
            leaf.evidence[0].locator.ends_with(&format!(
                "#entry=W1;method=ex-method 2018;report=EX-R 7%3B p. 3;date=2026-09-01;\
                 revision=sha256:{}",
                DIGEST.to_uppercase()
            )),
            "{}",
            leaf.evidence[0].locator
        );
        // Computed on another revision: stale, never read.
        let stale = catalogues.read("sound", "W2", "rw", &source());
        assert!(
            stale
                .value
                .as_ref()
                .is_err_and(|why| why.contains("is stale")
                    && why.contains("`rev-B`")
                    && why.contains(DIGEST)),
            "{stale:?}"
        );
        assert_eq!(
            catalogues.stale_results(),
            [StaleResult {
                catalogue: "sound".into(),
                key: "W2".into(),
                revision: Some("rev-B".into()),
                stale: true,
                why: format!(
                    "result `W2` of catalogue `sound` was computed on model revision `rev-B`, \
                     and the checked model is sha256:{DIGEST}"
                ),
            }]
        );
        // A revision the host states names the model too.
        let stated = sound(
            &csv,
            Some(ModelRevision::new(DIGEST, Some("rev-B".into())).unwrap()),
        );
        assert!(stated.read("sound", "W2", "rw", &source()).value.is_ok());
        assert!(stated.stale_results().is_empty());
    }

    #[test]
    fn an_untraceable_result_or_an_unknown_model_revision_is_never_read() {
        let csv = "wall,R'w,method,report,date,revision\n\
                   W1,53,ex-method 2018,,2026-09-01,rev-A\n\
                   W2,49,ex-method 2018,EX-R 7,2026-09-01,rev-A\n";
        let catalogues = sound(csv, None);
        let untraced = catalogues.read("sound", "W1", "rw", &source());
        assert!(
            untraced
                .value
                .as_ref()
                .is_err_and(|why| why.contains("states no report") && why.contains("traced")),
            "{untraced:?}"
        );
        let unknown = catalogues.read("sound", "W2", "rw", &source());
        assert!(
            unknown
                .value
                .as_ref()
                .is_err_and(|why| why.contains("is not stated")),
            "{unknown:?}"
        );
        let stale = catalogues.stale_results();
        assert_eq!(stale.len(), 2);
        assert!(!stale[0].stale && stale[0].revision.is_none());
        assert!(stale[1].stale && stale[1].why.contains("no checked model's revision"));
    }

    #[test]
    fn results_declarations_and_revisions_are_checked() {
        fn results_of(definition: &mut CatalogueDefinition) -> &mut CalculationResults {
            definition.results.as_mut().unwrap()
        }
        let checked = |edit: &dyn Fn(&mut CatalogueDefinition)| {
            let mut definition = results();
            edit(&mut definition);
            check_definition("sound", &definition)
        };
        assert_eq!(checked(&|_| {}), Ok(()));
        assert!(
            checked(&|d| results_of(d).report = "pages".into())
                .is_err_and(|why| why.contains("not declared"))
        );
        assert!(
            checked(&|d| results_of(d).date = "method".into())
                .is_err_and(|why| why.contains("not date or dateTime"))
        );
        assert!(
            checked(&|d| results_of(d).revision = "rw".into())
                .is_err_and(|why| why.contains("not string"))
        );
        assert!(
            checked(&|d| results_of(d).method = "wall".into())
                .is_err_and(|why| why.contains("is its key"))
        );
        assert!(
            checked(&|d| results_of(d).report = "method".into())
                .is_err_and(|why| why.contains("both method and report"))
        );
        assert!(ModelRevision::new("abc", None).is_err());
        assert!(ModelRevision::new(DIGEST, Some(" ".into())).is_err());
        let mut catalogues = Catalogues::declared(&[results()]);
        let revision = ModelRevision::of_bytes(b"model", None).unwrap();
        assert!(revision.identifies(&format!("sha256:{}", revision.sha256())));
        catalogues
            .supply_revision(source(), revision.clone())
            .unwrap();
        assert!(catalogues.supply_revision(source(), revision).is_err());
    }

    /// Design speeds per station range of two alignments.
    fn speeds() -> CatalogueDefinition {
        serde_json::from_value(serde_json::json!({
            "id": "speeds",
            "name": {"default": "Design speeds", "translations": {}},
            "key": "alignment",
            "columns": [
                {"id": "alignment", "kind": "string"},
                {"id": "from", "kind": "quantity", "unit": "m"},
                {"id": "to", "kind": "quantity", "unit": "m"},
                {"id": "speed", "kind": "quantity", "unit": "km/h"},
                {"id": "category", "kind": "string"},
            ],
            "ranges": {"from": "from", "to": "to"},
        }))
        .unwrap()
    }

    fn ranged(csv: &str) -> Result<Catalogues, String> {
        let definition = speeds();
        check_definition("speeds", &definition)?;
        let mut catalogues = Catalogues::declared(std::slice::from_ref(&definition));
        let supplied =
            SuppliedCatalogue::read(&definition, csv.as_bytes(), "speeds.csv", None, None)?;
        catalogues.supply("speeds", supplied)?;
        Ok(catalogues)
    }

    fn metres(lower: f64, upper: f64) -> Value {
        Value::Number {
            value: crate::expression::Interval { lower, upper },
            unit: parse_unit("m").unwrap().1,
        }
    }

    const SPEEDS: &str = "alignment,from,to,speed,category\n\
                          ex-axis-1,0,120,60,urban\n\
                          ex-axis-1,250,400,80,rural\n\
                          ex-axis-1,120,250,80,rural\n\
                          ex-axis-2,0,1000,100,rural\n";

    #[test]
    fn a_value_stated_per_station_range_is_read_at_a_station() {
        let catalogues = ranged(SPEEDS).unwrap();
        let read = |key: &str, at: Value, column: &str| {
            catalogues.read_at("speeds", key, &at, column, &source())
        };
        let leaf = read("ex-axis-1", metres(150.0, 150.0), "speed");
        let Ok(Value::Number { value, .. }) = &leaf.value else {
            panic!("{leaf:?}");
        };
        assert!((value.lower - 80.0 / 3.6).abs() < 1e-9, "{value:?}");
        assert_eq!(leaf.evidence.len(), 1);
        assert!(
            leaf.evidence[0]
                .locator
                .ends_with("#entry=ex-axis-1;range=120 m..250 m"),
            "{}",
            leaf.evidence[0].locator
        );
        // A range holds its start, never its end.
        assert_eq!(
            read("ex-axis-1", metres(120.0, 120.0), "category").value,
            Ok(Value::Text("rural".into()))
        );
        assert_eq!(
            read("ex-axis-1", metres(0.0, 0.0), "category").value,
            Ok(Value::Text("urban".into()))
        );
        // A station across two ranges stating one value reads it, citing both.
        let across = read("ex-axis-1", metres(240.0, 260.0), "speed");
        assert!(
            across.value.is_ok() && across.evidence.len() == 2,
            "{across:?}"
        );
        // Across ranges stating different values: undecided, never one picked.
        let straddling = read("ex-axis-1", metres(119.0, 121.0), "speed");
        assert!(
            straddling
                .value
                .as_ref()
                .is_err_and(|why| why.contains("undecided") && why.contains("0 m..120 m")),
            "{straddling:?}"
        );
        // Beyond every range, or at the last one's end: no range holds it.
        let beyond = read("ex-axis-1", metres(400.0, 400.0), "speed");
        assert!(
            beyond
                .value
                .as_ref()
                .is_err_and(|why| why.contains("states no range of `ex-axis-1` holding")),
            "{beyond:?}"
        );
        assert!(
            read("ex-axis-3", metres(1.0, 1.0), "speed")
                .value
                .is_err_and(|why| why.contains("has no entry whose `alignment` is `ex-axis-3`"))
        );
        // A position of another unit, and a plain read, are refused.
        assert!(
            read(
                "ex-axis-1",
                Value::Number {
                    value: crate::expression::Interval {
                        lower: 1.0,
                        upper: 1.0
                    },
                    unit: crate::expression::Unit::NONE
                },
                "speed"
            )
            .value
            .is_err()
        );
        assert!(
            catalogues
                .read("speeds", "ex-axis-1", "speed", &source())
                .value
                .is_err_and(|why| why.contains("read at a position"))
        );
    }

    #[test]
    fn ranges_are_checked_when_declared_and_supplied() {
        fn ranges(definition: &mut CatalogueDefinition) -> &mut CatalogueRanges {
            definition.ranges.as_mut().unwrap()
        }
        assert!(
            ranged(
                "alignment,from,to,speed,category\nex-axis-1,0,120,60,u\nex-axis-1,100,200,80,r\n"
            )
            .is_err_and(|why| why.contains("overlapping ranges 0 m..120 m and 100 m..200 m"))
        );
        assert!(
            ranged("alignment,from,to,speed,category\nex-axis-1,120,120,60,u\n")
                .is_err_and(|why| why.contains("not after it"))
        );
        assert!(
            ranged("alignment,from,to,speed,category\nex-axis-1,,120,60,u\n")
                .is_err_and(|why| why.contains("states no range"))
        );
        let checked = |edit: &dyn Fn(&mut CatalogueDefinition)| {
            let mut definition = speeds();
            edit(&mut definition);
            check_definition("speeds", &definition)
        };
        assert!(checked(&|d| ranges(d).to = "from".into()).is_err());
        assert!(checked(&|d| ranges(d).to = "category".into()).is_err());
        assert!(
            checked(&|d| ranges(d).to = "speed".into()).is_err_and(|why| why.contains("units"))
        );
        assert!(checked(&|d| ranges(d).from = "alignment".into()).is_err());
        assert!(checked(&|d| ranges(d).from = "station".into()).is_err());
    }
}
