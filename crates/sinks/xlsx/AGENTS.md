# `axioval-xlsx`

Spreadsheet workbooks (`.xlsx`) of a report's findings, not-evaluated
outcomes and report tables.

- Depends on `axioval-ir`, `rust_xlsxwriter` (no default features) and
  `thiserror` only. Never the engine, a source adapter or an IFC type: the
  host names the external identity scheme (`Options::external_id_scheme`).
- Fail closed: not-evaluated outcomes are rows of the `Findings` sheet,
  never left out, and an unknown table value is a shaded blank (with
  `not evaluated` as a number's exactness), never zero or an empty string.
- Never collapse an interval: both bounds are written as numbers and the
  exactness column says `bounded`. Numbers are numeric cells, never text.
- Deterministic: the caller supplies the creation time, sheet names derive
  from the report alone (`sheet_names`), rows follow report order. Never
  read the clock here.
- Spreadsheet limits refuse the export (`ExportError::Write`); never
  truncate a text or drop rows to fit.
- Run `cargo test -p axioval-xlsx`; `tests/xlsx.rs` reads the cells back
  from the archive's XML.
