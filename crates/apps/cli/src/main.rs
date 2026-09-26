//! Command-line validation of normalized Axioval packages.
//!
//! `validate` binds a ruleset without a model. `check` runs it over a model and
//! writes the report as JSON, and optionally as a BCF archive.
//!
//! Exit status is part of the automation contract; see [`Outcome`].

use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

use axioval::{
    bcf,
    engine::{EvidenceSession, IntegritySeverity, Runtime, SourceIntegrityServiceHandle, compile},
    ifc::import_ifc_session,
    ir::{DefinitionPackage, Report, RuleSetPackage},
};
use clap::{Parser, Subcommand};
use serde::Serialize;

#[derive(Parser)]
#[command(name = "axioval", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Strictly bind a ruleset to definitions and trusted capabilities.
    Validate {
        #[arg(long, required = true)]
        definitions: Vec<PathBuf>,
        #[arg(long)]
        ruleset: PathBuf,
    },
    /// Check a model against a ruleset.
    ///
    /// Exit status: 0 every rule was evaluated and nothing was found; 3 at
    /// least one finding; 4 no finding, but something was not evaluated, so
    /// the model has not passed; 1 the check could not run; 2 invalid usage.
    Check {
        /// The model to check: an IFC2X3 or IFC4 STEP file.
        #[arg(long)]
        model: PathBuf,
        #[arg(long, required = true)]
        definitions: Vec<PathBuf>,
        #[arg(long)]
        ruleset: PathBuf,
        /// Write the JSON result here instead of stdout.
        #[arg(long)]
        report: Option<PathBuf>,
        /// Also write the report as a BCF 2.1 archive.
        #[arg(long)]
        bcf: Option<PathBuf>,
        /// BCF topic author.
        #[arg(long, default_value = "axioval", requires = "bcf")]
        bcf_author: String,
        /// BCF topic date as an `xs:dateTime`. Defaults to `SOURCE_DATE_EPOCH`
        /// when set, else the current time, in UTC.
        #[arg(long, requires = "bcf")]
        bcf_date: Option<String>,
    },
}

/// How a completed run ends. Errors exit 1 and usage errors 2 (clap).
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// Every rule was evaluated and none found anything.
    Passed,
    /// At least one finding. Takes precedence over [`Outcome::Incomplete`]:
    /// a finding is conclusive whatever else was skipped.
    Findings,
    /// No finding, but part of the check could not be evaluated. Never a pass.
    Incomplete,
}
impl Outcome {
    fn of(report: &Report) -> Self {
        if !report.findings().is_empty() {
            Self::Findings
        } else if !report.not_evaluated().is_empty() {
            Self::Incomplete
        } else {
            Self::Passed
        }
    }
    fn code(&self) -> ExitCode {
        ExitCode::from(match self {
            Self::Passed => 0,
            Self::Findings => 3,
            Self::Incomplete => 4,
        })
    }
}

/// The JSON document `check` writes.
#[derive(Serialize)]
struct CheckOutput<'a> {
    report: &'a Report,
    /// Irregularities of the model itself, separate from rule findings.
    integrity: Vec<IntegrityRecord>,
}
#[derive(Serialize)]
struct IntegrityRecord {
    code: String,
    severity: &'static str,
    message: String,
    locator: String,
}

fn load<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, Box<dyn Error>> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?)
}

fn packages(
    definitions: &[PathBuf],
    ruleset: &Path,
) -> Result<(Vec<DefinitionPackage>, RuleSetPackage), Box<dyn Error>> {
    let definitions = definitions
        .iter()
        .map(|path| load(path))
        .collect::<Result<Vec<DefinitionPackage>, _>>()?;
    Ok((definitions, load(ruleset)?))
}

fn run() -> Result<Outcome, Box<dyn Error>> {
    match Cli::parse().command {
        Command::Validate {
            definitions,
            ruleset,
        } => {
            let (definitions, ruleset) = packages(&definitions, &ruleset)?;
            let plan = compile(&axioval::default_registry()?, &definitions, &ruleset)?;
            println!("validated {} executable rule(s)", plan.rules().len());
            Ok(Outcome::Passed)
        }
        Command::Check {
            model,
            definitions,
            ruleset,
            report,
            bcf,
            bcf_author,
            bcf_date,
        } => {
            let (definitions, ruleset) = packages(&definitions, &ruleset)?;
            let registry = axioval::default_registry()?;
            let plan = compile(&registry, &definitions, &ruleset)?;
            let session = import(&model)?;
            let result = Runtime::new(registry).run_session(&session, plan)?;
            let integrity = integrity(&session)?;

            let output = CheckOutput {
                report: &result,
                integrity,
            };
            let json = serde_json::to_string_pretty(&output)? + "\n";
            // Everything is built before anything is written, so a run that
            // fails leaves no partial output behind.
            let archive = match &bcf {
                Some(path) => {
                    let date = match bcf_date {
                        Some(date) => date,
                        None => timestamp()?,
                    };
                    let export = bcf::export(
                        &result,
                        session.project(),
                        &bcf::Options::new(bcf_author, date),
                    )?;
                    Some((path, export.to_bytes()?, export.unanchored))
                }
                None => None,
            };

            match &report {
                Some(path) => write(path, json.as_bytes())?,
                None => print!("{json}"),
            }
            if let Some((path, bytes, _)) = &archive {
                write(path, bytes)?;
            }
            for record in &output.integrity {
                eprintln!("{}: {}: {}", record.severity, record.code, record.message);
            }
            for object in archive.iter().flat_map(|(_, _, unanchored)| unanchored) {
                eprintln!(
                    "warning: {object} has no valid unique GlobalId; its BCF topic selects no component"
                );
            }
            eprintln!(
                "{} finding(s), {} not evaluated, {} integrity issue(s)",
                result.findings().len(),
                result.not_evaluated().len(),
                output.integrity.len()
            );
            Ok(Outcome::of(&result))
        }
    }
}

/// Imports the model, named by its file name so a report does not depend on
/// where the file was checked from.
fn import(model: &Path) -> Result<EvidenceSession, Box<dyn Error>> {
    let bytes = fs::read(model).map_err(|error| format!("{}: {error}", model.display()))?;
    let document = model
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{}: model path has no UTF-8 file name", model.display()))?;
    Ok(import_ifc_session(document, &bytes)
        .map_err(|error| format!("{}: {error}", model.display()))?)
}

fn integrity(session: &EvidenceSession) -> Result<Vec<IntegrityRecord>, Box<dyn Error>> {
    let Some(service) = session.service::<SourceIntegrityServiceHandle>() else {
        return Ok(vec![]);
    };
    let mut records = Vec::new();
    for snapshot in session.snapshots() {
        for issue in service.issues(snapshot.source())? {
            records.push(IntegrityRecord {
                code: issue.code,
                severity: match issue.severity {
                    IntegritySeverity::Warning => "warning",
                    IntegritySeverity::Error => "error",
                },
                message: issue.message,
                locator: issue.evidence.locator,
            });
        }
    }
    Ok(records)
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    Ok(fs::write(path, bytes).map_err(|error| format!("{}: {error}", path.display()))?)
}

/// UTC `xs:dateTime` of `SOURCE_DATE_EPOCH` when set, else of now.
///
/// `SOURCE_DATE_EPOCH` is the reproducible-builds convention: with it set,
/// the same model and ruleset write byte-identical BCF.
fn timestamp() -> Result<String, Box<dyn Error>> {
    let seconds = match std::env::var("SOURCE_DATE_EPOCH") {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|_| format!("SOURCE_DATE_EPOCH `{value}` is not a count of seconds"))?,
        Err(_) => SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
    };
    Ok(utc(seconds))
}

/// Formats seconds since the Unix epoch as `YYYY-MM-DDThh:mm:ssZ`.
fn utc(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86_400).unwrap_or(i64::MAX);
    let time = seconds % 86_400;
    // Howard Hinnant's days-to-civil conversion for the proleptic Gregorian calendar.
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time / 3_600,
        time % 3_600 / 60,
        time % 60
    )
}

fn main() -> ExitCode {
    match run() {
        Ok(outcome) => outcome.code(),
        Err(error) => {
            eprintln!("axioval: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::utc;

    #[test]
    fn utc_formats_known_instants() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(utc(1_790_416_799), "2026-09-26T09:59:59Z");
        assert_eq!(utc(4_107_542_400), "2100-03-01T00:00:00Z");
    }
}
