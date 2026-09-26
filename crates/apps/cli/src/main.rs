//! Command-line validation of normalized Axioval packages.
//!
//! `validate` binds a ruleset without a model. `check` runs it over a model and
//! writes the report as JSON, and optionally as a BCF archive. `report` reads
//! a saved result back as a bounded summary or a filtered, paged listing.
//!
//! Exit status is part of the automation contract; see [`Outcome`].

use std::{
    error::Error,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

mod digest;
mod geometry;

use axioval::{
    bcf,
    engine::{EvidenceSession, IntegritySeverity, Runtime, SourceIntegrityServiceHandle, compile},
    ifc::import_ifc_session,
    ir::{DefinitionPackage, Report, RuleSetPackage},
};
use clap::{Args, Parser, Subcommand};
use digest::{CheckOutput, Filter, IntegrityRecord, Section};

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
    Check(CheckArgs),
    /// Read a result saved by `check --report`.
    ///
    /// Without filters, prints the same bounded summary as `check --summary`.
    /// With any filter, lists the matching entries, paged.
    Report(ReportArgs),
}

#[derive(Args)]
struct CheckArgs {
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
    /// Mesh the model's bodies so geometric rules can run. Off by default:
    /// meshing costs time and purely semantic rulesets do not need it.
    #[arg(long)]
    geometry: bool,
    /// Print a bounded summary to stdout instead of the full JSON. Save the
    /// full result with `--report` to dig in with `axioval report`.
    #[arg(long)]
    summary: bool,
    /// Groups per section in the summary.
    #[arg(long, default_value_t = 10, requires = "summary")]
    top: usize,
}

#[derive(Args)]
struct ReportArgs {
    /// The JSON result `check --report` wrote.
    result: PathBuf,
    /// Only entries from this section.
    #[arg(long, value_enum)]
    section: Option<Section>,
    /// Only findings and not-evaluated outcomes of this rule.
    #[arg(long)]
    rule: Option<String>,
    /// Only integrity issues with this code.
    #[arg(long)]
    code: Option<String>,
    /// Only entries naming this object: a local id such as `#42`, a full
    /// id, or a `GlobalId`.
    #[arg(long)]
    object: Option<String>,
    /// Include evidence locators in listed entries.
    #[arg(long)]
    evidence: bool,
    #[arg(long, default_value_t = 20)]
    limit: usize,
    #[arg(long, default_value_t = 0)]
    offset: usize,
    /// Groups per section in the summary.
    #[arg(long, default_value_t = 10)]
    top: usize,
    /// Print JSON instead of text.
    #[arg(long)]
    json: bool,
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
        if report.has_findings() {
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
        Command::Check(args) => check(args),
        Command::Report(args) => {
            report(args)?;
            Ok(Outcome::Passed)
        }
    }
}

fn check(args: CheckArgs) -> Result<Outcome, Box<dyn Error>> {
    let (definitions, ruleset) = packages(&args.definitions, &args.ruleset)?;
    let registry = axioval::default_registry()?;
    let plan = compile(&registry, &definitions, &ruleset)?;
    let bytes =
        fs::read(&args.model).map_err(|error| format!("{}: {error}", args.model.display()))?;
    let session = import(&args.model, &bytes)?;
    let (session, meshed) = if args.geometry {
        let (session, report) = geometry::attach(session, &bytes)
            .map_err(|error| format!("{}: geometry: {error}", args.model.display()))?;
        (session, Some(report))
    } else {
        (session, None)
    };
    let result = Runtime::new(registry).run_session(&session, plan)?;
    let integrity = integrity(&session)?;

    let geometry = meshed.map(|report| digest::GeometryRecord {
        exact: report.exact,
        tessellated: report.tessellated,
        no_body: report.no_body,
        unmeasured: report
            .unmeasured
            .into_iter()
            .map(|(object, reason)| digest::Unmeasured { object, reason })
            .collect(),
    });
    let output = CheckOutput::new(result, integrity, geometry, session.project());
    let result = &output.report;
    let json = serde_json::to_string_pretty(&output)? + "\n";
    // Everything is built before anything is written, so a run that fails
    // leaves no partial output behind.
    let archive = match &args.bcf {
        Some(path) => {
            let date = match args.bcf_date {
                Some(date) => date,
                None => timestamp()?,
            };
            let export = bcf::export(
                result,
                session.project(),
                &bcf::Options::new(args.bcf_author, date),
            )?;
            Some((path, export.to_bytes()?, export.unanchored))
        }
        None => None,
    };
    let summary = args.summary.then(|| {
        let saved = args
            .report
            .as_deref()
            .map(|path| path.to_string_lossy().into_owned());
        digest::render_summary(&digest::summarize(&output, args.top, saved.as_deref()))
    });

    if let Some(path) = &args.report {
        write(path, json.as_bytes())?;
    }
    match &summary {
        Some(text) => print!("{text}"),
        None if args.report.is_none() => print!("{json}"),
        None => {}
    }
    if let Some((path, bytes, _)) = &archive {
        write(path, bytes)?;
    }
    let unanchored: Vec<_> = archive
        .iter()
        .flat_map(|(_, _, unanchored)| unanchored)
        .collect();
    warn(&output, summary.is_some(), &unanchored);
    Ok(Outcome::of(result))
}

fn report(args: ReportArgs) -> Result<(), Box<dyn Error>> {
    let output: CheckOutput = load(&args.result)?;
    let path = args.result.to_string_lossy();
    let filtered = args.section.is_some()
        || args.rule.is_some()
        || args.code.is_some()
        || args.object.is_some();
    let text = if filtered {
        let command = listing_command(&path, &args);
        let filter = Filter {
            section: args.section,
            rule: args.rule,
            code: args.code,
            object: args.object,
        };
        let listing = digest::list(
            &output,
            &filter,
            args.offset,
            args.limit,
            args.evidence,
            &command,
        );
        if args.json {
            serde_json::to_string_pretty(&listing)? + "\n"
        } else {
            digest::render_listing(&listing)
        }
    } else {
        let summary = digest::summarize(&output, args.top, Some(&path));
        if args.json {
            serde_json::to_string_pretty(&summary)? + "\n"
        } else {
            digest::render_summary(&summary)
        }
    };
    print!("{text}");
    Ok(())
}

/// Diagnostics for stderr. A summary already groups integrity issues and
/// states the counts, so with one only what it cannot show is repeated.
fn warn(output: &CheckOutput, summarized: bool, unanchored: &[&axioval::ir::ObjectId]) {
    if summarized {
        // The summary already groups integrity issues and states the counts;
        // one line each would repeat it at the size it exists to avoid.
        if !unanchored.is_empty() {
            eprintln!(
                "warning: {} object(s) have no valid unique GlobalId; their BCF topics select no component",
                unanchored.len()
            );
        }
    } else {
        for record in &output.integrity {
            eprintln!("{}: {}: {}", record.severity, record.code, record.message);
        }
        for object in unanchored {
            eprintln!(
                "warning: {object} has no valid unique GlobalId; its BCF topic selects no component"
            );
        }
        if let Some(geometry) = &output.geometry {
            eprintln!(
                "geometry: {} exact, {} tessellated, {} without body, {} unmeasured",
                geometry.exact,
                geometry.tessellated,
                geometry.no_body,
                geometry.unmeasured.len()
            );
            for unmeasured in &geometry.unmeasured {
                eprintln!(
                    "warning: {} was not meshed: {}",
                    unmeasured.object, unmeasured.reason
                );
            }
        }
        eprintln!(
            "{} finding(s), {} not evaluated, {} integrity issue(s)",
            output.report.findings().len() + output.report.rule_findings().len(),
            output.report.not_evaluated().len(),
            output.integrity.len()
        );
    }
}

/// The `report` invocation that repeats a listing, for its next-page hint.
fn listing_command(path: &str, args: &ReportArgs) -> String {
    let mut command = format!("axioval report {}", digest::shell_quote(path));
    if let Some(section) = args.section {
        let name = clap::ValueEnum::to_possible_value(&section)
            .map(|value| value.get_name().to_owned())
            .unwrap_or_default();
        let _ = write!(command, " --section {name}");
    }
    for (flag, value) in [
        ("--rule", &args.rule),
        ("--code", &args.code),
        ("--object", &args.object),
    ] {
        if let Some(value) = value {
            let _ = write!(command, " {flag} {}", digest::shell_quote(value));
        }
    }
    if args.evidence {
        command.push_str(" --evidence");
    }
    let _ = write!(command, " --limit {}", args.limit);
    if args.json {
        command.push_str(" --json");
    }
    command
}

/// Imports the model, named by its file name so a report does not depend on
/// where the file was checked from.
fn import(model: &Path, bytes: &[u8]) -> Result<EvidenceSession, Box<dyn Error>> {
    let document = model
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{}: model path has no UTF-8 file name", model.display()))?;
    Ok(import_ifc_session(document, bytes)
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
                }
                .to_owned(),
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
