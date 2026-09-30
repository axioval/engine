//! IDS documents as rule packages: `check --ids`, `ids translate`, and
//! `ids export`, which writes rule packages back as IDS and is
//! `export --profile ids` by another name.
//!
//! A document is translated in memory by `axioval-ids`. Only complete
//! specifications run: one with any gap runs none of its rules, since a
//! partial translation would report a specification as met that was never
//! wholly checked. Every gap is reported with its specification, and the
//! check can then never exit 0.

use std::{error::Error, fs, path::Path};

use axioval::ir::{DefinitionPackage, RuleSetPackage, contract::Selector};
use axioval_ids::{Options, translate};
use clap::{Args, Subcommand};

use crate::digest::{IdsRecord, IdsSpecification};
use crate::export::ExportArgs;

/// The `ids` subcommands.
#[derive(Subcommand)]
pub(crate) enum IdsCommand {
    /// Translate an IDS document into a definition package and a ruleset.
    ///
    /// Writes the packages `check --ids` would run, for inspection or for
    /// `check --definitions --ruleset`. A specification with a translation
    /// gap is left out and listed with its gaps on stderr. Exit status: 0
    /// every specification translated, 4 the packages were written without
    /// the specifications listed, 1 nothing written, 2 invalid usage.
    Translate(TranslateArgs),
    /// Export a ruleset's alphanumerical rules as an IDS 1.0 document.
    ///
    /// A rule is exported only when IDS states it exactly: a folder
    /// translated from IDS as the specification it came from, any other
    /// rule as one specification (an entity with the facets its selector
    /// states, and one property, attribute, classification, material or
    /// part-of requirement). Every other rule is listed on stderr with why
    /// it is not exported. Exit status: 0 every rule exported, 4 the
    /// document was written without the rules listed, 1 nothing written
    /// (no rule exportable, an unreadable package), 2 invalid usage. The
    /// same as `export --profile ids`.
    Export(ExportArgs),
}

#[derive(Args)]
pub(crate) struct TranslateArgs {
    /// The IDS 1.0 document.
    ids: std::path::PathBuf,
    /// Where to write the definition package.
    #[arg(long, value_name = "FILE")]
    definitions: std::path::PathBuf,
    /// Where to write the ruleset.
    #[arg(long, value_name = "FILE")]
    ruleset: std::path::PathBuf,
    /// Restrict every specification by the selector in this JSON file; see
    /// `check --ids-filter`.
    #[arg(long, value_name = "FILE")]
    ids_filter: Option<std::path::PathBuf>,
}

/// An IDS document translated into the packages that run.
pub(crate) struct Translated {
    pub definitions: DefinitionPackage,
    pub ruleset: RuleSetPackage,
    /// What became of each specification.
    pub record: IdsRecord,
}

impl Translated {
    /// Whether every specification runs.
    pub fn is_complete(&self) -> bool {
        self.record
            .specifications
            .iter()
            .all(|specification| specification.gaps.is_empty())
    }
}

/// Reads and translates the IDS document at `path`, every specification
/// restricted by the selector in the file `filter` when given, leaving out
/// every specification with a gap, and lists those on stderr.
pub(crate) fn load(path: &Path, filter: Option<&Path>) -> Result<Translated, Box<dyn Error>> {
    let filter: Option<Selector> = filter.map(crate::load).transpose()?;
    let shown = || path.display().to_string();
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", shown()))?;
    let document =
        openbim_ids::from_slice(&bytes).map_err(|error| format!("{}: {error}", shown()))?;
    let mut options = Options::new(package_id(path), "0.0.0");
    if let Some(filter) = &filter {
        options = options.with_filter(filter.clone());
    }
    let translation =
        translate(&document, &options).map_err(|error| format!("{}: {error}", shown()))?;
    let mut ruleset = translation.ruleset;
    let mut specifications = Vec::with_capacity(translation.specifications.len());
    for outcome in translation.specifications {
        let gaps: Vec<String> = outcome.gaps.iter().map(ToString::to_string).collect();
        let rules = if gaps.is_empty() {
            outcome.rules
        } else {
            // A specification with a gap runs none of its rules.
            ruleset.root.folders.retain(|folder| {
                !folder
                    .rules
                    .iter()
                    .any(|rule| outcome.rules.contains(&rule.id))
            });
            Vec::new()
        };
        specifications.push(IdsSpecification {
            number: outcome.number,
            name: outcome.name,
            rules,
            gaps,
        });
    }
    let record = IdsRecord {
        document: path
            .file_name()
            .map_or_else(shown, |name| name.to_string_lossy().into_owned()),
        filter,
        specifications,
    };
    report_gaps(&record);
    Ok(Translated {
        definitions: translation.definitions,
        ruleset,
        record,
    })
}

/// Lists every specification left out, with its gaps, on stderr.
fn report_gaps(record: &IdsRecord) {
    let mut skipped = 0;
    for specification in &record.specifications {
        if specification.gaps.is_empty() {
            continue;
        }
        skipped += 1;
        eprintln!(
            "ids: {}: specification {} {:?} is not checked:",
            record.document, specification.number, specification.name
        );
        for gap in &specification.gaps {
            eprintln!("ids:   {gap}");
        }
    }
    if skipped > 0 {
        eprintln!(
            "ids: {}: {skipped} of {} specification(s) not checked",
            record.document,
            record.specifications.len()
        );
    }
}

/// `ids:` and the file stem in lower case, every other character a `-`,
/// such as `ids:fire-safety` for `Fire Safety.ids`.
fn package_id(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let mut name = String::with_capacity(stem.len());
    for character in stem.chars() {
        let character = if character.is_ascii_alphanumeric() || "._".contains(character) {
            character
        } else {
            '-'
        };
        if !(character == '-' && (name.is_empty() || name.ends_with('-'))) {
            name.push(character);
        }
    }
    let name = name.trim_end_matches('-');
    format!("ids:{}", if name.is_empty() { "document" } else { name })
}

/// `ids translate`: writes both packages, or neither.
pub(crate) fn translate_command(args: &TranslateArgs) -> Result<bool, Box<dyn Error>> {
    let translated = load(&args.ids, args.ids_filter.as_deref())?;
    let definitions = serde_json::to_string_pretty(&translated.definitions)? + "\n";
    let ruleset = serde_json::to_string_pretty(&translated.ruleset)? + "\n";
    crate::write(&args.definitions, definitions.as_bytes())?;
    crate::write(&args.ruleset, ruleset.as_bytes())?;
    let specifications = &translated.record.specifications;
    println!(
        "translated {} of {} specification(s) into {} rule(s)",
        specifications
            .iter()
            .filter(|specification| specification.gaps.is_empty())
            .count(),
        specifications.len(),
        specifications
            .iter()
            .map(|specification| specification.rules.len())
            .sum::<usize>()
    );
    Ok(translated.is_complete())
}

/// `ids export`: `export --profile ids`.
pub(crate) fn export_command(args: &ExportArgs) -> Result<bool, Box<dyn Error>> {
    crate::export::export_command(axioval_ids::IdsProfile::ID, args)
}

#[cfg(test)]
mod tests {
    use super::package_id;
    use std::path::Path;

    #[test]
    fn the_package_id_is_the_file_stem() {
        assert_eq!(
            package_id(Path::new("dir/Fire Safety.ids")),
            "ids:fire-safety"
        );
        assert_eq!(package_id(Path::new("a__b.v2.ids")), "ids:a__b.v2");
        assert_eq!(package_id(Path::new("  .ids")), "ids:document");
        assert_eq!(package_id(Path::new("§§x§.ids")), "ids:x");
    }
}
