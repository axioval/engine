//! Command-line validation of normalized Axioval packages.
//!
//! `validate` binds a ruleset without a model. `check` runs it over one or
//! more models, each a source of one session with an optional discipline, and
//! writes the report as JSON, and optionally as a BCF archive. `compare`
//! compares two revisions of a model object by object and writes the same
//! kind of result. `report` reads a saved result back as a bounded summary
//! or a filtered, paged listing. `decide` records a reviewer's decisions
//! about a saved result's findings, which `check --decisions` carries over
//! to a re-check. `bcf push` and `bcf pull` exchange a saved result's
//! topics and decisions with a BCF API 3.0 server.
//!
//! Exit status is part of the automation contract; see [`Outcome`].

use std::{
    collections::BTreeMap,
    error::Error,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

mod compare;
mod digest;
mod geometry;
mod ids;
mod server;

use axioval::{
    bcf, bcf_snapshot,
    engine::{
        DisciplineMap, DisciplineOrigin, DisciplineRule, EvidenceSession, IntegritySeverity,
        LocationMethod, LocationPolicy, QUALIFIED_RULE_SEPARATOR, Runtime,
        SourceIntegrityServiceHandle, SourceMetadata, UnmappedReason, compile_rulesets,
    },
    ifc,
    ir::{
        DateTime, Decision, DecisionComment, DecisionStatus, Decisions, DefinitionPackage,
        Discipline, FindingId, ObjectId, Project, Report, RuleSetPackage, SourceId,
        contract::SourceField,
    },
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
        /// A ruleset; repeat to bind several together, as `check` does.
        #[arg(long = "ruleset", required = true)]
        rulesets: Vec<PathBuf>,
    },
    /// Check a model against a ruleset.
    ///
    /// Exit status: 0 every rule was evaluated and nothing was found; 3 at
    /// least one finding; 4 no finding, but something was not evaluated, so
    /// the model has not passed; 1 the check could not run; 2 invalid usage.
    Check(CheckArgs),
    /// Compare two revisions of a model, object by object.
    ///
    /// Objects are matched by `GlobalId`. Kind, classifications, named
    /// properties, placement and the coordinate system are compared, and
    /// with `--geometry` each object's measured bounds. Exit status as for
    /// `check`: 0 identical, 3 at least one difference, 4 no difference but
    /// something could not be compared, 1 the comparison could not run, 2
    /// invalid usage.
    Compare(compare::CompareArgs),
    /// Read a result saved by `check --report` or `compare --report`.
    ///
    /// Without filters, prints the same bounded summary as `check --summary`.
    /// With any filter, lists the matching entries, paged.
    Report(ReportArgs),
    /// Work with buildingSMART IDS documents.
    Ids {
        #[command(subcommand)]
        command: ids::IdsCommand,
    },
    /// Record a reviewer's decision about findings of a saved result.
    ///
    /// Accepts, rejects or reopens each `--finding` (its `id` in the
    /// result) in the decisions file, creating it when missing and
    /// replacing an earlier decision about the same finding. `check
    /// --decisions` carries them over to a re-check. Exit status: 0
    /// recorded, 1 nothing written (an unknown finding, an unreadable
    /// file), 2 invalid usage.
    Decide(DecideArgs),
    /// Exchange topics with a BCF API 3.0 server.
    Bcf {
        #[command(subcommand)]
        command: server::BcfCommand,
    },
}

#[derive(Args)]
struct CheckArgs {
    /// A model to check: an IFC2X3, IFC4 or IFC4X3 STEP file, or an ifcZIP
    /// archive holding exactly one `.ifc` member, optionally followed by
    /// `:DISCIPLINE`, the role it plays (`arch.ifc:architecture`). Repeat for
    /// several models; each is one source of the check, named by its file
    /// name. A discipline is a lowercase token (`a-z`, `0-9`, `-`, `_`). A
    /// file whose name itself ends in `:name` takes a trailing `:`.
    #[arg(long = "model", required = true, value_name = "PATH[:DISCIPLINE]", value_parser = model_arg)]
    models: Vec<ModelArg>,
    /// Assign a discipline to each model declaring none from what the file
    /// states: `FIELD:PATTERN=DISCIPLINE`, where FIELD is `application`,
    /// `fileName`, `project` or `schema` and PATTERN a wildcard pattern
    /// (`*`, `?`, `\` escapes) over the whole value, case-sensitive
    /// (`application:*Architecture*=architecture`). Repeat for several;
    /// the first matching one assigns. The result lists where each
    /// discipline came from.
    #[arg(long = "discipline-map", value_name = "FIELD:PATTERN=DISCIPLINE", value_parser = discipline_rule)]
    discipline_map: Vec<DisciplineRule>,
    #[arg(long, required_unless_present = "ids")]
    definitions: Vec<PathBuf>,
    /// A ruleset to check. Repeat for several: each is compiled against
    /// its own definition packages and its rule ids are qualified by its
    /// package id (`package-id/rule-id`), so two rulesets may both define
    /// a rule `r1`. With one ruleset the ids stay as written.
    #[arg(long = "ruleset", required_unless_present = "ids")]
    rulesets: Vec<PathBuf>,
    /// Check against a buildingSMART IDS 1.0 document instead of packages:
    /// it is translated in memory, as `axioval ids translate` writes it. A
    /// specification that cannot be translated exactly runs none of its
    /// rules and is listed with its gaps on stderr and in the result's
    /// `ids` field; the check then never exits 0.
    #[arg(long, value_name = "FILE", conflicts_with_all = ["definitions", "rulesets"])]
    ids: Option<PathBuf>,
    /// Restrict every IDS specification to part of the model: a JSON
    /// selector, written in IFC names, combined with each specification's
    /// applicability through `allOf` (such as the walls contained in one
    /// storey). The result records it.
    #[arg(long, value_name = "FILE", requires = "ids")]
    ids_filter: Option<PathBuf>,
    /// Mesh the model's bodies so geometric rules can run. Off by default:
    /// meshing costs time and purely semantic rulesets do not need it.
    #[arg(long)]
    geometry: bool,
    /// Locate every finding and not-evaluated outcome by storey and space:
    /// `storeys` climbs the spatial containment to storeys, `containers` to
    /// storeys and spaces, `geometry` takes spaces from the bodies that
    /// contain or meet each object (with `--geometry`). Off by default,
    /// and then the result is unchanged.
    #[arg(long, value_enum, default_value_t = Locate::None)]
    locate: Locate,
    /// Report each rule's counts and status: how many objects it surely
    /// selected, how many it found or left not evaluated, and whether it
    /// passed, failed, was not evaluated or selected nothing. The summary
    /// then lists them. Off by default, and then the result is unchanged.
    #[arg(long)]
    rule_status: bool,
    /// Carry the decisions in this file (written by `axioval decide`) over
    /// to the findings with the same identity; decisions whose finding is
    /// gone are listed as stale. Never changes the exit status.
    #[arg(long, value_name = "FILE")]
    decisions: Option<PathBuf>,
    /// Carry the review state of the topics in this BCF 2.1 or 3.0 archive
    /// (as `--bcf` wrote it, then reviewed in another BCF tool) over to the
    /// findings whose identity is the topic GUID: a closed, resolved, done
    /// or accepted topic accepts its finding, a rejected one rejects it, and
    /// comments are carried. Topics that decide no current finding are
    /// listed in the result's `unmatched_topics`. Never changes the exit
    /// status.
    #[arg(long, value_name = "FILE", conflicts_with = "decisions")]
    decisions_from: Option<PathBuf>,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Args)]
struct DecideArgs {
    /// The JSON result `check --report` wrote.
    result: PathBuf,
    /// The decisions file to update; created when it does not exist.
    #[arg(long, value_name = "FILE")]
    decisions: PathBuf,
    /// A finding's `id` in the result. Repeat to decide several alike.
    #[arg(long = "finding", required = true, value_name = "ID")]
    findings: Vec<FindingId>,
    #[arg(long, value_enum)]
    status: StatusArg,
    /// Who decides.
    #[arg(long)]
    author: String,
    /// Why. Added to the end of the finding's thread: an earlier
    /// decision's comments are kept.
    #[arg(long, default_value = "")]
    comment: String,
    /// Who the finding is assigned to.
    #[arg(long, value_name = "NAME")]
    assign_to: Option<String>,
    /// When the finding is due, an ISO 8601 date-time with offset.
    #[arg(long, value_name = "DATE-TIME")]
    due: Option<DateTime>,
    /// How urgent, in the project's vocabulary (such as `High`).
    #[arg(long)]
    priority: Option<String>,
    /// A label; repeat for several. Replaces the earlier decision's labels.
    #[arg(long = "label", value_name = "LABEL")]
    labels: Vec<String>,
    /// When, an ISO 8601 date-time with offset. Defaults to
    /// `SOURCE_DATE_EPOCH` when set, else the current time, in UTC.
    #[arg(long)]
    date: Option<DateTime>,
}

/// The `decide --status` values.
#[derive(Clone, Copy, clap::ValueEnum)]
enum StatusArg {
    Accepted,
    Rejected,
    Open,
}

impl From<StatusArg> for DecisionStatus {
    fn from(status: StatusArg) -> Self {
        match status {
            StatusArg::Accepted => Self::Accepted,
            StatusArg::Rejected => Self::Rejected,
            StatusArg::Open => Self::Open,
        }
    }
}

/// How `check --locate` locates outcomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum Locate {
    None,
    Storeys,
    Containers,
    Geometry,
}

impl Locate {
    /// The IFC location policy: storeys and spaces by class, climbed to
    /// along spatial containment and aggregation, named by `Name`.
    fn policy(self) -> Option<LocationPolicy> {
        let method = match self {
            Self::None => return None,
            Self::Storeys => LocationMethod::Storeys,
            Self::Containers => LocationMethod::Containers,
            Self::Geometry => LocationMethod::Geometry,
        };
        Some(LocationPolicy {
            method,
            storey_kinds: vec!["IfcBuildingStorey".to_owned()],
            space_kinds: vec!["IfcSpace".to_owned()],
            containment: vec![
                "IfcRelContainedInSpatialStructure:backward".to_owned(),
                "IfcRelAggregates:backward".to_owned(),
            ],
            name: Some((axioval::ir::ATTRIBUTE_SET.to_owned(), "Name".to_owned())),
        })
    }
}

/// Where and how a result is written; shared by `check` and `compare`.
#[derive(Args)]
struct OutputArgs {
    /// Write the JSON result here instead of stdout.
    #[arg(long)]
    report: Option<PathBuf>,
    /// Also write the report as a BCF archive.
    #[arg(long)]
    bcf: Option<PathBuf>,
    /// BCF version. 3.0 needs a camera on every viewpoint, so it needs
    /// `--geometry` and bounds for every selected object; otherwise nothing
    /// is written and the run fails.
    #[arg(long, value_enum, default_value = "2.1", requires = "bcf")]
    bcf_version: BcfVersion,
    /// BCF topic author.
    #[arg(long, default_value = "axioval", requires = "bcf")]
    bcf_author: String,
    /// BCF topic date as an `xs:dateTime`. Defaults to `SOURCE_DATE_EPOCH`
    /// when set, else the current time, in UTC.
    #[arg(long, requires = "bcf")]
    bcf_date: Option<String>,
    /// How BCF viewpoints show the involved objects.
    #[command(flatten)]
    bcf_view: BcfViewArgs,
    /// Print a bounded summary to stdout instead of the full JSON. Save the
    /// full result with `--report` to dig in with `axioval report`.
    #[arg(long)]
    summary: bool,
    /// Groups per section in the summary.
    #[arg(long, default_value_t = 10, requires = "summary")]
    top: usize,
}

/// How BCF viewpoints show the involved objects: colouring, visibility and
/// a section box.
#[derive(Args)]
#[allow(clippy::struct_excessive_bools)] // Each is one independent flag.
struct BcfViewArgs {
    /// Colour of each BCF viewpoint's subject, as `RRGGBB` or `AARRGGBB`
    /// hex digits [default: FFFF0000]. Colouring is written with
    /// `--geometry` or when a colour is given.
    #[arg(long = "bcf-subject-color", value_name = "HEX", requires = "bcf")]
    subject_color: Option<bcf::Color>,
    /// Colour of each BCF viewpoint's related objects [default: FF0000FF].
    #[arg(long = "bcf-related-color", value_name = "HEX", requires = "bcf")]
    related_color: Option<bcf::Color>,
    /// Write no colouring in BCF viewpoints, even with `--geometry`.
    #[arg(
        long = "bcf-no-color",
        requires = "bcf",
        conflicts_with_all = ["subject_color", "related_color"]
    )]
    no_color: bool,
    /// Show only the involved objects in each BCF viewpoint, hiding the
    /// rest of the model.
    #[arg(long = "bcf-isolate", requires = "bcf")]
    isolate: bool,
    /// Cut each BCF viewpoint with a fitted camera by a section box around
    /// its objects' measured bounds. Needs `--geometry` to have bounds;
    /// viewpoints without a camera are never clipped.
    #[arg(long = "bcf-section-box", requires = "bcf")]
    section_box: bool,
    /// Add a PNG snapshot to each BCF viewpoint with a camera, rendered
    /// from the meshed bodies. Illustrative, never evidence: every topic
    /// with one says so. Needs `--geometry`.
    #[arg(long = "bcf-snapshots", requires = "bcf")]
    snapshots: bool,
}

/// The `--bcf-version` values.
#[derive(Clone, Copy, clap::ValueEnum)]
enum BcfVersion {
    #[value(name = "2.1")]
    V2_1,
    #[value(name = "3.0")]
    V3_0,
}

impl From<BcfVersion> for bcf::Version {
    fn from(version: BcfVersion) -> Self {
        match version {
            BcfVersion::V2_1 => Self::V2_1,
            BcfVersion::V3_0 => Self::V3_0,
        }
    }
}

#[derive(Args)]
struct ReportArgs {
    /// The JSON result `check --report` wrote.
    result: PathBuf,
    /// Only entries from this section.
    #[arg(long, value_enum)]
    section: Option<Section>,
    /// Only findings, not-evaluated outcomes and table rows of this rule.
    #[arg(long)]
    rule: Option<String>,
    /// Only integrity issues with this code.
    #[arg(long)]
    code: Option<String>,
    /// Only entries naming this object: a local id such as `#42`, a full
    /// id, or a `GlobalId`.
    #[arg(long)]
    object: Option<String>,
    /// Only findings and not-evaluated outcomes located in this storey or
    /// space (by name, local id, full id or `GlobalId`), as `check
    /// --locate` located them. An outcome whose location is unresolved is
    /// kept: it may lie there.
    #[arg(long)]
    location: Option<String>,
    /// Only findings with this decision, as `check --decisions` carried it
    /// over: `accepted`, `rejected`, `open`, `undecided`, or `changed`
    /// (decided, and changed since).
    #[arg(long, value_enum)]
    decision: Option<digest::DecisionFilter>,
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
    /// Print one report table as CSV: the one `--rule` and `--table`
    /// select.
    #[arg(long, conflicts_with_all = ["json", "section", "code", "object", "location"])]
    csv: bool,
    /// With `--csv`: the table's name, such as `takeoff`.
    #[arg(long, requires = "csv")]
    table: Option<String>,
}

/// One `--model` argument: a file and the discipline declared for it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ModelArg {
    path: PathBuf,
    discipline: Option<Discipline>,
}

/// Parses `PATH[:DISCIPLINE]`.
///
/// The discipline is the text after the last `:` when that text is a
/// discipline name, so a Windows drive (`C:\m.ifc`) or a directory with a
/// colon stays part of the path. A trailing `:` declares no discipline and
/// keeps everything before it as the path. Text after the last `:` that
/// looks like a name but is not a valid one (`m.ifc:Structure`) is refused
/// rather than read as part of a file name.
fn model_arg(value: &str) -> Result<ModelArg, String> {
    let whole = || ModelArg {
        path: PathBuf::from(value),
        discipline: None,
    };
    let Some((path, suffix)) = value.rsplit_once(':') else {
        return Ok(whole());
    };
    if path.is_empty() {
        return Ok(whole());
    }
    if suffix.is_empty() {
        return Ok(ModelArg {
            path: PathBuf::from(path),
            discipline: None,
        });
    }
    let name_like = suffix
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if !name_like {
        return Ok(whole());
    }
    let discipline = Discipline::new(suffix).map_err(|error| error.to_string())?;
    Ok(ModelArg {
        path: PathBuf::from(path),
        discipline: Some(discipline),
    })
}

/// Parses `FIELD:PATTERN=DISCIPLINE`.
///
/// The field is everything before the first `:` and the discipline
/// everything after the last `=`, so the pattern may hold both.
fn discipline_rule(value: &str) -> Result<DisciplineRule, String> {
    let usage = || format!("`{value}` is not FIELD:PATTERN=DISCIPLINE");
    let (field, rest) = value.split_once(':').ok_or_else(usage)?;
    let (pattern, discipline) = rest.rsplit_once('=').ok_or_else(usage)?;
    let field = match field {
        "application" => SourceField::Application,
        "fileName" => SourceField::FileName,
        "project" => SourceField::Project,
        "schema" => SourceField::Schema,
        "timestamp" => SourceField::Timestamp,
        other => {
            return Err(format!(
                "`{other}` is not `application`, `fileName`, `project`, `schema` or `timestamp`"
            ));
        }
    };
    let discipline = Discipline::new(discipline).map_err(|error| error.to_string())?;
    DisciplineRule::new(field, pattern, discipline).map_err(|error| error.to_string())
}

/// Every source of the session with its discipline and where it came from.
fn source_infos(session: &EvidenceSession) -> Vec<digest::SourceInfo> {
    session
        .snapshots()
        .map(|snapshot| {
            let source = snapshot.source();
            let origin = session.discipline_origin(source);
            let (mapped_by, mapped_value) = match origin {
                Some(DisciplineOrigin::Mapped { rule, value, .. }) => {
                    (Some(rule.clone()), Some(value.clone()))
                }
                _ => (None, None),
            };
            digest::SourceInfo {
                source: source.to_string(),
                discipline: session.discipline(source).map(ToString::to_string),
                discipline_origin: origin.map(|origin| {
                    match origin {
                        DisciplineOrigin::Declared => "declared",
                        DisciplineOrigin::Mapped { .. } => "mapped",
                    }
                    .to_owned()
                }),
                mapped_by,
                mapped_value,
                unmapped: session.unmapped(source).map(|reason| match reason {
                    UnmappedReason::NoMatch => "no rule of the discipline map matches".to_owned(),
                    UnmappedReason::Unread(rule) => {
                        format!("the model does not state what `{rule}` reads")
                    }
                }),
            }
        })
        .collect()
}

/// How a completed run ends. Errors exit 1 and usage errors 2 (clap).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
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

fn load<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, Box<dyn Error>> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?)
}

fn packages(
    definitions: &[PathBuf],
    rulesets: &[PathBuf],
) -> Result<(Vec<DefinitionPackage>, Vec<RuleSetPackage>), Box<dyn Error>> {
    let definitions = definitions
        .iter()
        .map(|path| load(path))
        .collect::<Result<Vec<DefinitionPackage>, _>>()?;
    let rulesets = rulesets
        .iter()
        .map(|path| load(path))
        .collect::<Result<Vec<RuleSetPackage>, _>>()?;
    Ok((definitions, rulesets))
}

fn run() -> Result<Outcome, Box<dyn Error>> {
    match Cli::parse().command {
        Command::Validate {
            definitions,
            rulesets,
        } => {
            let (definitions, rulesets) = packages(&definitions, &rulesets)?;
            let plan = compile_rulesets(&axioval::default_registry()?, &definitions, &rulesets)?;
            println!("validated {} executable rule(s)", plan.rules().len());
            Ok(Outcome::Passed)
        }
        Command::Check(args) => check(args),
        Command::Compare(args) => compare::compare(args),
        Command::Ids {
            command: ids::IdsCommand::Translate(args),
        } => Ok(if ids::translate_command(&args)? {
            Outcome::Passed
        } else {
            Outcome::Incomplete
        }),
        Command::Ids {
            command: ids::IdsCommand::Export(args),
        } => Ok(if ids::export_command(&args)? {
            Outcome::Passed
        } else {
            Outcome::Incomplete
        }),
        Command::Report(args) => {
            report(args)?;
            Ok(Outcome::Passed)
        }
        Command::Decide(args) => {
            decide(&args)?;
            Ok(Outcome::Passed)
        }
        Command::Bcf { command } => {
            server::run(command)?;
            Ok(Outcome::Passed)
        }
    }
}

/// Records `args`'s decision about each of its findings, taking each
/// finding's basis from the result. Nothing is written unless every
/// finding is in the result.
fn decide(args: &DecideArgs) -> Result<(), Box<dyn Error>> {
    let output: CheckOutput = load(&args.result)?;
    let mut decisions: Decisions = if args.decisions.exists() {
        load(&args.decisions)?
    } else {
        Decisions::default()
    };
    let date = match args.date {
        Some(date) => date,
        None => i64::try_from(epoch_seconds()?)
            .ok()
            .and_then(DateTime::from_unix_seconds)
            .ok_or("the current time is not a date-time in the years 0000 to 9999")?,
    };
    for id in &args.findings {
        let finding = output
            .report
            .findings()
            .iter()
            .find(|finding| finding.id == Some(*id))
            .ok_or_else(|| format!("{}: no finding has id {id}", args.result.display()))?;
        let mut decision =
            Decision::new(*id, args.status.into(), &args.author, date)?.with_basis(finding);
        // What the reviewer does not restate is kept from the earlier
        // decision: an assignment outlives a status change, and a comment
        // continues the thread.
        let earlier = decisions.get(*id);
        decision.comments = earlier.map(|e| e.comments.clone()).unwrap_or_default();
        let comment = args.comment.trim();
        if !comment.is_empty() {
            decision
                .comments
                .push(DecisionComment::new(args.author.trim(), date, comment));
        }
        decision.assigned_to = args
            .assign_to
            .as_deref()
            .map(|name| name.trim().to_owned())
            .or_else(|| earlier.and_then(|e| e.assigned_to.clone()));
        decision.due_date = args.due.or_else(|| earlier.and_then(|e| e.due_date));
        decision.priority = args
            .priority
            .as_deref()
            .map(|priority| priority.trim().to_owned())
            .or_else(|| earlier.and_then(|e| e.priority.clone()));
        decision.labels = if args.labels.is_empty() {
            earlier.map(|e| e.labels.clone()).unwrap_or_default()
        } else {
            args.labels
                .iter()
                .map(|label| label.trim().to_owned())
                .collect()
        };
        decisions.record(decision)?;
    }
    let json = serde_json::to_string_pretty(&decisions)? + "\n";
    write(&args.decisions, json.as_bytes())?;
    println!(
        "recorded {} decision(s) in {}",
        args.findings.len(),
        args.decisions.display()
    );
    Ok(())
}

fn check(args: CheckArgs) -> Result<Outcome, Box<dyn Error>> {
    let translated = args
        .ids
        .as_deref()
        .map(|path| ids::load(path, args.ids_filter.as_deref()))
        .transpose()?;
    let (definitions, rulesets) = match &translated {
        Some(translated) => (
            vec![translated.definitions.clone()],
            vec![translated.ruleset.clone()],
        ),
        None => packages(&args.definitions, &args.rulesets)?,
    };
    let registry = axioval::default_registry()?;
    let plan = compile_rulesets(&registry, &definitions, &rulesets)?;
    if args.locate == Locate::Geometry && !args.geometry {
        return Err("`--locate geometry` needs `--geometry` to derive spaces from bodies".into());
    }
    let labels = rule_labels(&rulesets);
    let decisions: Option<Decisions> = args.decisions.as_deref().map(load).transpose()?;
    let reviewed: Option<Vec<u8>> = args
        .decisions_from
        .as_deref()
        .map(|path| fs::read(path).map_err(|error| format!("{}: {error}", path.display())))
        .transpose()?;
    let (session, bytes) = sources(&args.models)?;
    let session = session.with_discipline_map(
        &args
            .discipline_map
            .iter()
            .cloned()
            .fold(DisciplineMap::new(), DisciplineMap::with),
    );
    let (session, mut meshed) = if args.geometry {
        let (session, report) = geometry::attach(session, &bytes, args.output.bcf_view.snapshots)
            .map_err(|error| format!("geometry: {error}"))?;
        (session, Some(report))
    } else {
        (session, None)
    };
    let mut runtime = Runtime::new(registry);
    if let Some(policy) = args.locate.policy() {
        runtime = runtime.with_locations(policy);
    }
    if args.rule_status {
        runtime = runtime.with_rule_summaries();
    }
    let mut result = runtime.run_session(&session, plan)?;
    // Keyed as the BCF sink keys its topics, so a decision recorded against
    // either is the same.
    result.identify_findings(session.project(), bcf::IFC_GLOBAL_ID_SCHEME)?;
    if let Some(decisions) = &decisions {
        result.apply_decisions(decisions)?;
    }
    let mut unmatched = Vec::new();
    if let (Some(bytes), Some(path)) = (&reviewed, &args.decisions_from) {
        let imported = bcf::import(bytes, &result, session.project(), &labels)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        result.apply_decisions(&imported.decisions)?;
        unmatched = imported.unmatched.into_iter().map(Into::into).collect();
    }
    let integrity = integrity(&session)?;

    let bodies = meshed
        .as_mut()
        .map(|report| std::mem::take(&mut report.meshes));
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
    let mut output = CheckOutput::new(result, integrity, geometry, session.project())
        .with_sources(source_infos(&session))
        .with_unmatched_topics(unmatched);
    let complete = translated.as_ref().is_none_or(ids::Translated::is_complete);
    if let Some(translated) = translated {
        output = output.with_ids(translated.record);
    }
    let bounds = args
        .geometry
        .then(|| geometry::bounds(&[&session], &output.report));
    emit(
        &output,
        session.project(),
        (bounds, bodies),
        labels,
        args.output,
    )?;
    Ok(match Outcome::of(&output.report) {
        // A specification that did not run was not checked: never a pass.
        Outcome::Passed if !complete => Outcome::Incomplete,
        outcome => outcome,
    })
}

/// The BCF labels of every rule: folder path and tags, keyed by rule id as
/// the report names it, qualified by package when several rulesets run.
fn rule_labels(rulesets: &[RuleSetPackage]) -> BTreeMap<String, Vec<String>> {
    let qualify = rulesets.len() > 1;
    let mut labels = BTreeMap::new();
    for ruleset in rulesets {
        for (rule, own) in bcf::ruleset_labels(ruleset) {
            let rule = if qualify {
                format!("{}{QUALIFIED_RULE_SEPARATOR}{rule}", ruleset.package.id)
            } else {
                rule
            };
            labels.insert(rule, own);
        }
    }
    labels
}

/// Writes a result as `args` asks: JSON to stdout or `--report`, a summary,
/// a BCF archive, and diagnostics to stderr. `bounds` are the measured
/// objects' extents with `--geometry`, which fit the BCF cameras;
/// `rule_labels` label each rule's topics (see [`rule_labels`]).
///
/// Everything is built before anything is written, so a run that fails
/// leaves no partial output behind.
/// Each meshed object's triangles, to draw BCF snapshots from.
pub(crate) type Meshes = BTreeMap<ObjectId, bcf_snapshot::Mesh>;

pub(crate) fn emit(
    output: &CheckOutput,
    project: &Project,
    (bounds, meshes): (Option<BTreeMap<ObjectId, bcf::Bounds>>, Option<Meshes>),
    rule_labels: BTreeMap<String, Vec<String>>,
    args: OutputArgs,
) -> Result<(), Box<dyn Error>> {
    let json = serde_json::to_string_pretty(output)? + "\n";
    let archive = match &args.bcf {
        Some(path) => {
            let date = match args.bcf_date {
                Some(date) => date,
                None => timestamp()?,
            };
            // Coloured with geometry, which frames the objects, or when asked;
            // without either the archive stays as it was before colouring.
            let colored = !args.bcf_view.no_color
                && (bounds.is_some()
                    || args.bcf_view.subject_color.is_some()
                    || args.bcf_view.related_color.is_some());
            let defaults = bcf::Colors::default();
            let colors = colored.then(|| bcf::Colors {
                subject: args.bcf_view.subject_color.unwrap_or(defaults.subject),
                related: args.bcf_view.related_color.unwrap_or(defaults.related),
            });
            let options = bcf::Options {
                version: args.bcf_version.into(),
                colors,
                isolate: args.bcf_view.isolate,
                section_box: args.bcf_view.section_box,
                bounds,
                rule_labels,
                ..bcf::Options::new(args.bcf_author, date)
            };
            let export = if args.bcf_view.snapshots {
                let meshes = meshes.ok_or("`--bcf-snapshots` needs `--geometry` to have meshes")?;
                let export = bcf::export_with_snapshots(
                    &output.report,
                    project,
                    &options,
                    &bcf_snapshot::Renderer::new(meshes),
                )?;
                if !export.unrendered.is_empty() {
                    eprintln!(
                        "warning: {} BCF viewpoint subject(s) have no mesh; their viewpoints have no snapshot",
                        export.unrendered.len()
                    );
                }
                export
            } else {
                bcf::export(&output.report, project, &options)?
            };
            Some((path, export.to_bytes()?, export.unanchored, export.unframed))
        }
        None => None,
    };
    let summary = args.summary.then(|| {
        let saved = args
            .report
            .as_deref()
            .map(|path| path.to_string_lossy().into_owned());
        digest::render_summary(&digest::summarize(output, args.top, saved.as_deref()))
    });

    if let Some(path) = &args.report {
        write(path, json.as_bytes())?;
    }
    match &summary {
        Some(text) => print!("{text}"),
        None if args.report.is_none() => print!("{json}"),
        None => {}
    }
    if let Some((path, bytes, _, _)) = &archive {
        write(path, bytes)?;
    }
    let unanchored: Vec<_> = archive
        .iter()
        .flat_map(|(_, _, unanchored, _)| unanchored)
        .collect();
    let unframed: Vec<_> = archive
        .iter()
        .flat_map(|(_, _, _, unframed)| unframed)
        .collect();
    warn(output, summary.is_some(), &unanchored, &unframed);
    Ok(())
}

fn report(args: ReportArgs) -> Result<(), Box<dyn Error>> {
    let output: CheckOutput = load(&args.result)?;
    if args.csv {
        print!("{}", table_csv(&output, &args)?);
        return Ok(());
    }
    let path = args.result.to_string_lossy();
    let filtered = args.section.is_some()
        || args.rule.is_some()
        || args.code.is_some()
        || args.object.is_some()
        || args.location.is_some()
        || args.decision.is_some();
    let text = if filtered {
        let command = listing_command(&path, &args);
        let filter = Filter {
            section: args.section,
            rule: args.rule,
            code: args.code,
            object: args.object,
            location: args.location,
            decision: args.decision,
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

/// The one table `--rule` and `--table` select, as CSV; naming none or
/// several is an error listing the candidates.
fn table_csv(output: &CheckOutput, args: &ReportArgs) -> Result<String, Box<dyn Error>> {
    let tables: Vec<_> = output
        .report
        .tables()
        .iter()
        .filter(|table| {
            args.rule
                .as_deref()
                .is_none_or(|rule| rule == table.rule_id().to_string())
                && args
                    .table
                    .as_deref()
                    .is_none_or(|name| name == table.name())
        })
        .collect();
    match tables.as_slice() {
        [table] => Ok(digest::table_csv(table)),
        tables => {
            let names: Vec<String> = if tables.is_empty() {
                output.report.tables().iter().collect::<Vec<_>>()
            } else {
                tables.to_vec()
            }
            .iter()
            .map(|table| format!("--rule {} --table {}", table.rule_id(), table.name()))
            .collect();
            Err(format!(
                "--csv needs exactly one table, but {} tables match; candidates: {}",
                tables.len(),
                if names.is_empty() {
                    "none, the result has no tables".to_owned()
                } else {
                    names.join("; ")
                }
            )
            .into())
        }
    }
}

/// Diagnostics for stderr. A summary already groups integrity issues and
/// states the counts, so with one only what it cannot show is repeated.
fn warn(output: &CheckOutput, summarized: bool, unanchored: &[&ObjectId], unframed: &[&ObjectId]) {
    if !output.unmatched_topics.is_empty() {
        eprintln!(
            "warning: {} BCF topic(s) decided no current finding; see the result's unmatched_topics",
            output.unmatched_topics.len()
        );
    }
    if summarized {
        // The summary already groups integrity issues and states the counts;
        // one line each would repeat it at the size it exists to avoid.
        if !unanchored.is_empty() {
            eprintln!(
                "warning: {} object(s) have no valid unique GlobalId; their BCF topics select no component",
                unanchored.len()
            );
        }
        if !unframed.is_empty() {
            eprintln!(
                "warning: {} object(s) have no measured bounds; their BCF viewpoints have no camera",
                unframed.len()
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
        for object in unframed {
            eprintln!("warning: {object} has no measured bounds; its BCF viewpoint has no camera");
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
            output.report.findings().len(),
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
        ("--location", &args.location),
    ] {
        if let Some(value) = value {
            let _ = write!(command, " {flag} {}", digest::shell_quote(value));
        }
    }
    if let Some(decision) = args.decision {
        let name = clap::ValueEnum::to_possible_value(&decision)
            .map(|value| value.get_name().to_owned())
            .unwrap_or_default();
        let _ = write!(command, " --decision {name}");
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

/// Imports every model as one source of one session, with its discipline.
///
/// Returns the session and each source's bytes, for meshing. Two models with
/// the same file name would be the same source, so they are refused.
fn sources(models: &[ModelArg]) -> Result<(EvidenceSession, geometry::ModelBytes), Box<dyn Error>> {
    let mut members = Vec::with_capacity(models.len());
    let mut bytes = geometry::ModelBytes::new();
    let mut paths: BTreeMap<SourceId, &Path> = BTreeMap::new();
    for model in models {
        let path = model.path.as_path();
        let (mut session, content) = import(path, None)?;
        let source = session
            .snapshots()
            .next()
            .map(|snapshot| snapshot.source().clone())
            .ok_or_else(|| format!("{}: the import produced no source", path.display()))?;
        if let Some(first) = paths.insert(source.clone(), path) {
            return Err(format!(
                "{} and {} share the file name `{}`, which names the source; rename one",
                first.display(),
                path.display(),
                source.document
            )
            .into());
        }
        session = session
            .with_source_metadata(
                &source,
                SourceMetadata::new().with(SourceField::FileName, [source.document.clone()]),
            )
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if let Some(discipline) = &model.discipline {
            session = session
                .with_discipline(&source, discipline.clone())
                .map_err(|error| format!("{}: {error}", path.display()))?;
        }
        bytes.insert(source, content);
        members.push(session);
    }
    Ok((EvidenceSession::federate(members)?, bytes))
}

/// Imports the model at `path`, a STEP file or an ifcZIP archive, as the
/// source `document`, by default its file name, so a report does not depend
/// on where the file was checked from. An archive's source is named by the
/// archive and its model member (`m.ifczip/m.ifc`).
///
/// Returns the session and the STEP bytes read, an archive's member's, for
/// meshing.
pub(crate) fn import(
    path: &Path,
    document: Option<&str>,
) -> Result<(EvidenceSession, Vec<u8>), Box<dyn Error>> {
    let document = match document {
        Some(document) => document,
        None => path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("{}: model path has no UTF-8 file name", path.display()))?,
    };
    let content = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let is_archive = ifc::is_ifc_zip(&content)
        || path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case(ifc::IFC_ZIP_EXTENSION));
    let (document, content) = if is_archive {
        let member =
            ifc::read_ifc_zip(&content).map_err(|error| format!("{}: {error}", path.display()))?;
        (member.document(document), member.into_bytes())
    } else {
        (document.to_owned(), content)
    };
    let session = ifc::import_ifc_session(document, &content)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok((session, content))
}

pub(crate) fn integrity(session: &EvidenceSession) -> Result<Vec<IntegrityRecord>, Box<dyn Error>> {
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

pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    Ok(fs::write(path, bytes).map_err(|error| format!("{}: {error}", path.display()))?)
}

/// UTC `xs:dateTime` of `SOURCE_DATE_EPOCH` when set, else of now.
///
/// `SOURCE_DATE_EPOCH` is the reproducible-builds convention: with it set,
/// the same model and ruleset write byte-identical BCF.
fn timestamp() -> Result<String, Box<dyn Error>> {
    Ok(utc(epoch_seconds()?))
}

/// Seconds since the Unix epoch of `SOURCE_DATE_EPOCH` when set, else of now.
fn epoch_seconds() -> Result<u64, Box<dyn Error>> {
    Ok(match std::env::var("SOURCE_DATE_EPOCH") {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|_| format!("SOURCE_DATE_EPOCH `{value}` is not a count of seconds"))?,
        Err(_) => SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
    })
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
    use super::{ModelArg, model_arg, utc};
    use axioval::ir::Discipline;
    use std::path::PathBuf;

    fn parsed(path: &str, discipline: Option<&str>) -> ModelArg {
        ModelArg {
            path: PathBuf::from(path),
            discipline: discipline.map(|name| Discipline::new(name).unwrap()),
        }
    }

    #[test]
    fn a_model_argument_splits_off_its_discipline() {
        assert_eq!(model_arg("m.ifc"), Ok(parsed("m.ifc", None)));
        assert_eq!(
            model_arg("dir/m.ifc:structure"),
            Ok(parsed("dir/m.ifc", Some("structure")))
        );
        assert_eq!(
            model_arg("m.ifc:building_services"),
            Ok(parsed("m.ifc", Some("building_services")))
        );
        // A drive letter or a colon inside the path is not a discipline.
        assert_eq!(
            model_arg("C:\\models\\m.ifc"),
            Ok(parsed("C:\\models\\m.ifc", None))
        );
        assert_eq!(
            model_arg("C:\\m.ifc:mep"),
            Ok(parsed("C:\\m.ifc", Some("mep")))
        );
        assert_eq!(model_arg("a:b/m.ifc"), Ok(parsed("a:b/m.ifc", None)));
        // A trailing colon keeps a file name that ends in `:name`.
        assert_eq!(model_arg("odd:arch:"), Ok(parsed("odd:arch", None)));
        assert_eq!(model_arg(":arch"), Ok(parsed(":arch", None)));
    }

    #[test]
    fn a_discipline_that_is_not_a_lowercase_token_is_refused() {
        assert!(model_arg("m.ifc:Structure").is_err());
        assert!(model_arg("m.ifc:-structure").is_err());
    }

    #[test]
    fn utc_formats_known_instants() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(utc(1_790_416_799), "2026-09-26T09:59:59Z");
        assert_eq!(utc(4_107_542_400), "2100-03-01T00:00:00Z");
    }
}
