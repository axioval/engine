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

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_ir::contract::{
    CatalogueDefinition, ColumnKind, ParameterValue, ScalarValue, TableFileColumn, TableRow,
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
    Ok(())
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
}

/// The catalogues of a run: every one the plan declares, with the entries
/// the host supplied for it. Installed in the run's services (as
/// `Arc<Catalogues>`), where every expression reads them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalogues {
    declared: BTreeMap<String, CatalogueDefinition>,
    supplied: BTreeMap<String, Indexed>,
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
        self.supplied
            .insert(id.to_owned(), Indexed { supplied, entries });
        Ok(())
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
        let Some(Indexed { supplied, entries }) = self.supplied.get(catalogue) else {
            return Leaf::unreadable(format!(
                "catalogue `{catalogue}` takes its entries from the host at check time, and \
                 none were supplied, so its entry `{key}` cannot be read"
            ));
        };
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
        let evidence = Evidence::exact(
            source.clone(),
            format!(
                "{CATALOGUE_LOCATOR_PREFIX};id={catalogue}{version};sha256={}#entry={key}",
                supplied.sha256
            ),
        );
        let value = match supplied.rows[index].get(column) {
            None => Ok(Value::Null),
            Some(cell) => ScalarValue::try_from(cell.clone())
                .map_err(|_| format!("catalogue `{catalogue}` column `{column}` states no value"))
                .and_then(|scalar| Value::from_literal(&scalar)),
        };
        Leaf {
            value,
            evidence: vec![evidence],
        }
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
}
