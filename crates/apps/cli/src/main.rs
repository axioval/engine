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
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

mod bounds;
mod compare;
mod digest;
mod draft;
mod export;
mod ids;
mod results;
mod server;
mod stats;
mod terrain;
mod timings;
mod zones;

/// The binary's allocator (#407). The system's keeps what each thread
/// freed in arenas of its own and rarely hands it back, so resident memory
/// stayed near a check's heaviest step and grew with the threads; this one
/// returns freed pages to the system. Only the binary sets it.
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

// The IFC to Axiolid geometry bridge (`--geometry`), from the host layer.
use axioval::ifc_geometry as geometry;

use axioval::{
    bcf, bcf_snapshot,
    engine::{
        DisciplineMap, DisciplineOrigin, DisciplineRule, EvidenceSession, ExecutionPlan,
        GeometryDemand, IntegritySeverity, LocationMethod, LocationPolicy,
        QUALIFIED_RULE_SEPARATOR, Runtime, SourceIntegrityServiceHandle, SourceMetadata,
        SuppliedCatalogue, SuppliedPairs, UnmappedReason, compile_rulesets,
    },
    html, ifc,
    ir::{
        DateTime, Decision, DecisionComment, DecisionStatus, Decisions, DefinitionPackage,
        Discipline, FindingId, ObjectId, Project, Report, RuleSetPackage, SourceId,
        contract::SourceField,
    },
    xlsx,
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
    /// Strictly bind a ruleset to definitions and trusted capabilities, or
    /// check one rule or expression draft within them.
    ///
    /// With `--rule` or `--expression`, prints the draft's diagnostics as
    /// JSON (code, message, expression path, JSON pointer, suggestion) and,
    /// with `--model`, how it judges each object with its traces; with
    /// `--serve`, answers drafts as JSON lines. Exit status: 0 valid, 3 the
    /// draft is refused, 1 nothing could be checked, 2 invalid usage.
    Validate(draft::ValidateArgs),
    /// Print the authoring catalogue: every capability with its
    /// parameters, the measured values, expression node kinds, operators,
    /// selector kinds, relationships and units, labelled in English and
    /// German, as versioned JSON a rule editor is generated from.
    ///
    /// With `--definitions`, it also lists each package's concept
    /// vocabulary. Exit status: 0 written, 1 nothing written.
    Catalogue {
        /// A definition package: JSON, or an MCS `definitions` archive.
        /// Repeat for several.
        #[arg(long)]
        definitions: Vec<PathBuf>,
        /// Write to this file instead of standard output.
        #[arg(long, value_name = "FILE")]
        output: Option<PathBuf>,
        /// Write every text in one language (`en` or `de`) instead of all:
        /// a text missing in it falls back to English, and each that did
        /// is listed on stderr.
        #[arg(long, value_name = "LOCALE")]
        locale: Option<String>,
    },
    /// Check a model against a ruleset.
    ///
    /// Exit status: 0 every rule was evaluated and nothing was found; 3 at
    /// least one finding; 4 no finding, but something was not evaluated, so
    /// the model has not passed; 1 the check could not run; 2 invalid usage.
    Check(Box<CheckArgs>),
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
    /// Export a ruleset as another format, through an export profile.
    ///
    /// A rule is exported only when the format states exactly what decides
    /// it; every other rule is listed on stderr with why it is not
    /// exported, and any structure or presentation the format reduces is
    /// listed as degraded. Profiles: `ids` (as `ids export`). Exit status:
    /// 0 nothing lost, 4 the document was written with the losses listed, 1
    /// nothing written (no rule exportable, an unknown profile, an
    /// unreadable package), 2 invalid usage.
    Export(export::ProfileExportArgs),
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
#[allow(clippy::struct_excessive_bools)] // Each is one independent flag.
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
    /// A definition package the rulesets bind: JSON, or an MCS
    /// `definitions` archive (`.mcs`, a template library). Repeat for
    /// several. A ruleset archive carries its own definitions; those it
    /// lacks are taken from these.
    #[arg(long)]
    definitions: Vec<PathBuf>,
    /// A ruleset to check: JSON, or an MCS `ruleset` archive (`.mcs`) with
    /// the definitions it carries. Repeat for several: each is compiled
    /// against its own definition packages and its rule ids are qualified
    /// by its package id (`package-id/rule-id`), so two rulesets may both
    /// define a rule `r1`. With one ruleset the ids stay as written.
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
    /// Supply the pairs of a relation the rulesets declare `by: supplied`
    /// (project data kept beside the model, not in the rule package): a
    /// CSV file, or one sheet of an xlsx workbook (`FILE.xlsx#SHEET`),
    /// read by the columns the relation declares. Repeat for several
    /// relations. A file for a relation no ruleset declares as supplied,
    /// or one that cannot be read, stops the check before anything runs; a
    /// supplied relation given no file leaves every rule walking it not
    /// evaluated. The result records each file's SHA-256.
    #[arg(long = "relations", value_name = "RELATION=FILE[#SHEET]", value_parser = relation_arg)]
    relations: Vec<RelationArg>,
    /// Supply the entries of a catalogue the rulesets declare (material or
    /// product data, such as each material's thermal conductivity, kept
    /// beside the model, not in the rule package): a CSV file, or one sheet
    /// of an xlsx workbook (`FILE.xlsx#SHEET`), read by the columns the
    /// catalogue declares, with the catalogue's version after an `@`.
    /// Repeat for several catalogues. A file for a catalogue no ruleset
    /// declares, or one that cannot be read, stops the check before
    /// anything runs; a catalogue given no file leaves every object whose
    /// expression reads it not evaluated. Every value read cites the
    /// catalogue, its version and the entry, and the result records each
    /// file's SHA-256.
    #[arg(
        long = "catalogue",
        value_name = "CATALOGUE[@VERSION]=FILE[#SHEET]",
        value_parser = catalogue_arg
    )]
    catalogues: Vec<CatalogueArg>,
    /// State the revision of a checked model, for the results of external
    /// calculations a results catalogue holds: `DOCUMENT=REVISION` names
    /// the model by its file name, and a bare `REVISION` the one model
    /// checked. A model's revision is always also the SHA-256 of its file
    /// (`sha256:<hex>`); a result computed on another revision is stale,
    /// reported as an integrity warning and never read.
    #[arg(long = "model-revision", value_name = "[DOCUMENT=]REVISION")]
    model_revisions: Vec<String>,
    /// Supply context zones beside the model (parcels, zoning areas,
    /// building limits and lines): a `GeoJSON` feature collection naming its
    /// reference system in `crs` (`axioval:model` for the model's own plan
    /// coordinates, or a map system the model's map conversion names), each
    /// feature a zone with an `id`, a `kind`, an `accuracy` in metres and a
    /// polygon, multipolygon or line string; or a GML document of cadastral
    /// parcels and public traffic areas, or a zoning plan (`XPlanGML`) with
    /// the values its areas state, which states no accuracy, so the
    /// argument does: `ACCURACY=FILE`, in metres. Repeat for several files.
    /// A file that cannot be read, or a zone id two files state, stops the
    /// check before anything runs; a zone the rules read and no file states
    /// leaves every value reading it not evaluated. The result records each
    /// file's SHA-256, its format, and per model whether its reference
    /// system and coordinates match the model's.
    #[arg(long = "zones", value_name = "[ACCURACY=]FILE", value_parser = zones::zone_arg)]
    zones: Vec<zones::ZoneArg>,
    /// Supply a terrain surface as context beside the models: which ground
    /// it is (`original` or `finished`), its vertical accuracy in metres
    /// (the most any of its elevations may differ from the true ground;
    /// `LandXML` states none, so the argument must), and a `LandXML` file with
    /// one TIN surface, or the one named after `#`. Repeat for the other
    /// ground. Heights above the terrain are measured against it as
    /// intervals widened by the accuracy. A file that cannot be read stops
    /// the check before anything runs; the result records each file's
    /// SHA-256.
    #[arg(
        long = "terrain",
        value_name = "KIND@ACCURACY=FILE[#SURFACE]",
        value_parser = terrain::terrain_arg,
        requires = "terrain_in"
    )]
    terrains: Vec<terrain::TerrainArg>,
    /// The coordinates the `--terrain` files are in: the models' own
    /// (`model`), or the map each model's stated map conversion maps onto
    /// (`map`); a model stating no map conversion then leaves every height
    /// above the terrain not evaluated.
    #[arg(long = "terrain-in", value_enum, requires = "terrains")]
    terrain_in: Option<terrain::TerrainIn>,
    /// Mesh the model's bodies so geometric rules can run. Off by default:
    /// meshing costs time and purely semantic rulesets do not need it.
    /// Only the bodies some rule can ask about are meshed: none for rules
    /// that read properties only, the selected spaces, obstacles and doors
    /// for a free-floor rule that selects its obstacles, every body
    /// otherwise (see `--mesh-all`).
    #[arg(long)]
    geometry: bool,
    /// With `--geometry`, mesh every object's body, whatever the rules can
    /// ask about, so the result's `geometry` record and the model-data
    /// warnings cover the whole model. The report is the same either way.
    #[arg(long, requires = "geometry")]
    mesh_all: bool,
    /// With `--geometry`, mesh only: build no exact boundaries. By default a
    /// body whose construction is exact (a rigidly placed extrusion or
    /// revolution of a rectangle, circle, section or line-and-arc profile,
    /// or a disk swept along a segment or an arc) also gets its exact
    /// boundary, so distances between curved bodies are certified
    /// instead of widened by the chord deviation.
    #[arg(long, requires = "geometry")]
    no_exact_boundaries: bool,
    /// With `--geometry` and several `--model`s, declare that every model
    /// shares the first one's frame as delivered. A model whose file states
    /// no frame to compare (no `Model` representation context, or a map
    /// conversion on one side only) or states it ambiguously is then
    /// measured in the first model's frame, and the result names it. A
    /// model whose stated frame differs beyond the tolerance stays
    /// unmeasured. Off by default.
    #[arg(long, requires = "geometry")]
    declare_shared_frame: bool,
    /// With `--geometry`, declare that one model was derived from another,
    /// naming both by file name: `DERIVED=ORIGINAL` (a model exported from
    /// another keeping copies of its objects, or another export of one
    /// authoring model with different exchange settings). Each object of
    /// DERIVED matched to one of ORIGINAL whose body is the original's
    /// within `--derived-tolerance` is superseded by it: evaluated once,
    /// through the original, and listed in the result's `superseded`.
    /// Moved, reshaped or unmatched objects stay evaluated. Repeat for
    /// several derived models.
    #[arg(
        long = "derived-from",
        value_name = "DERIVED=ORIGINAL",
        requires = "geometry"
    )]
    derived_from: Vec<String>,
    /// How `--derived-from` matches objects: `id` by `GlobalId` and class,
    /// the body confirmed; `id-or-geometry` also pairs an object with no
    /// such counterpart by its body alone when exactly one object of each
    /// side matches.
    #[arg(long, value_enum, default_value_t = DerivedMatch::Id, requires = "derived_from")]
    derived_match: DerivedMatch,
    /// The tolerance, in metres, within which `--derived-from` matches
    /// bodies in the shared world frame.
    #[arg(long, value_name = "METRES", default_value_t = geometry::DEFAULT_TOLERANCE_METRES, requires = "derived_from")]
    derived_tolerance: f64,
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
    /// Report how long each rule took: each rule's status (as with
    /// `--rule-status`) then carries its wall time and the time spent in
    /// the host services it called (geometry, proximity, routes), per
    /// service, and the summary lists the slowest rules. Timings are
    /// diagnostics: they never change a finding, its identity or the exit
    /// status.
    #[arg(long)]
    rule_timings: bool,
    /// Evaluate each rule's subjects on this many threads. Without it,
    /// `AXIOVAL_THREADS` names the count, and without that the machine's
    /// available parallelism does, up to 4. The result is the same on any
    /// number; the memory the heavy steps take is bounded by
    /// `--memory-budget`, not by the threads.
    #[arg(long, value_name = "N")]
    threads: Option<std::num::NonZeroUsize>,
    /// Evaluate the rules in another order wherever their dependencies
    /// leave it free: `plan` (the default), `reversed` or `shuffled:SEED`.
    /// A diagnostic of the determinism gate (#441): the result is the same
    /// in every order.
    #[arg(long, value_name = "ORDER", default_value = "plan", value_parser = rule_order, hide = true)]
    rule_order: axioval::engine::RuleOrder,
    /// Keep nothing built for reuse: no derivation, measurement or shared
    /// selection of the run, and no placement scene or footprint of the
    /// geometry services. A diagnostic of the determinism gate (#441): the
    /// result is the same, only slower.
    #[arg(long, hide = true)]
    no_reuse: bool,
    /// Keep at most this much of what the run's derivation store builds,
    /// all kinds together, such as `1` or `64M`: past it, the values read
    /// least recently are released and built again when read again. A
    /// diagnostic of the determinism gate (#440): the result is the same
    /// under any bound, only slower.
    #[arg(long, value_name = "SIZE", value_parser = axioval::engine::memory::parse_size, hide = true)]
    keep_derivations: Option<u64>,
    /// How many expression nodes each rule may evaluate, and the rules
    /// together before the rest are refused (100000000 unless given). A
    /// diagnostic of the determinism gate (#447): a budget that runs out
    /// refuses alike in every order, on any threads and without reuse.
    #[arg(long, value_name = "NODES", hide = true)]
    evaluation_budget: Option<u64>,
    /// How much memory the check's heavy geometry steps may take at once,
    /// such as `8G` or `512M` (powers of 1024), or `none`. A step that
    /// would pass it beside the ones running waits for one to finish; a
    /// step estimated at more than the whole budget is not evaluated, by
    /// name. Without it, `AXIOVAL_MEMORY_BUDGET` names it, and without that
    /// it is half the memory the system reports available, at least 6G.
    /// The result is the same under any budget that refuses no step.
    #[arg(long, value_name = "SIZE")]
    memory_budget: Option<String>,
    /// Print how long each phase took on stderr, each as it ends: compiling,
    /// reading the models, each geometry phase, each rule, and writing the
    /// outputs. Never changes the result or the exit status.
    #[arg(long)]
    timings: bool,
    /// Print on stderr, at the end of the check, how it reused what it
    /// built: per derivation kind, its version, hits, builds, waits, build
    /// time, the most bytes held at once, and the values released and
    /// evicted, as tab-separated lines starting with `stats`. Never changes
    /// the result or the exit status.
    #[arg(long)]
    stats: bool,
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

/// How `check --derived-from` matches objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum DerivedMatch {
    Id,
    IdOrGeometry,
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
    /// Also write the findings, not-evaluated outcomes and report tables as
    /// a spreadsheet workbook (xlsx): a `Findings` sheet and one sheet per
    /// table, numbers as numeric cells with their units in the header, an
    /// interval as its two bounds, and each value's exactness beside it.
    /// Created at `SOURCE_DATE_EPOCH` when set, else now.
    #[arg(long, value_name = "FILE")]
    xlsx: Option<PathBuf>,
    /// Also write the report as one self-contained HTML file: a cover, the
    /// summary, rules, categories, findings with their locations and
    /// decisions, not-evaluated outcomes and report tables, laid out to
    /// print (a browser prints it as a PDF). Dated `SOURCE_DATE_EPOCH`
    /// when set, else now.
    #[arg(long, value_name = "FILE")]
    html: Option<PathBuf>,
    /// The HTML report's template instead of the built-in one: HTML with
    /// placeholders such as `{{summary}}`, substituted and never run. It
    /// must place `{{summary}}` and `{{not-evaluated}}`.
    #[arg(long, value_name = "FILE", requires = "html")]
    html_template: Option<PathBuf>,
    /// The HTML report's title.
    #[arg(
        long,
        value_name = "TEXT",
        default_value = "Check report",
        requires = "html"
    )]
    html_title: String,
    /// `--html-template`, parsed before the run by [`OutputArgs::prepare`].
    #[arg(skip)]
    template: Option<html::Template>,
    /// Print a bounded summary to stdout instead of the full JSON. Save the
    /// full result with `--report` to dig in with `axioval report`.
    #[arg(long)]
    summary: bool,
    /// Groups per section in the summary.
    #[arg(long, default_value_t = 10, requires = "summary")]
    top: usize,
}

impl OutputArgs {
    /// Reads and parses `--html-template`, so a missing or invalid template
    /// fails before any work.
    pub(crate) fn prepare(&mut self) -> Result<(), Box<dyn Error>> {
        if let Some(path) = &self.html_template {
            let text =
                fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
            self.template = Some(
                html::Template::parse(&text)
                    .map_err(|error| format!("{}: {error}", path.display()))?,
            );
        }
        Ok(())
    }
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
    /// Frame each BCF viewpoint of a finding that says where it happens
    /// (the witness of a clash, clearance or distance) at that spot rather
    /// than at its objects' bounds, and list the witness in its
    /// description. Findings without one are framed as before.
    #[arg(long = "bcf-frame-witness", requires = "bcf")]
    frame_witness: bool,
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

/// One `--relations` argument: a supplied relation's id and the file
/// holding its pairs, with the workbook sheet to read.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RelationArg {
    relation: String,
    path: PathBuf,
    sheet: Option<String>,
}

/// Parses `RELATION=FILE`, `RELATION=FILE.xlsx#SHEET`.
///
/// The relation id ends at the first `=`. A `#` names a sheet only after a
/// path ending in `.xlsx`, so a CSV path may hold one.
fn relation_arg(value: &str) -> Result<RelationArg, String> {
    let Some((relation, file)) = value.split_once('=') else {
        return Err("expected RELATION=FILE".into());
    };
    if relation.is_empty() || file.is_empty() {
        return Err("expected RELATION=FILE, both non-empty".into());
    }
    let (path, sheet) = match file.rsplit_once('#') {
        Some((path, sheet))
            if path.to_ascii_lowercase().ends_with(".xlsx") && !sheet.is_empty() =>
        {
            (path, Some(sheet.to_owned()))
        }
        _ => (file, None),
    };
    Ok(RelationArg {
        relation: relation.to_owned(),
        path: PathBuf::from(path),
        sheet,
    })
}

/// One `--catalogue` argument: a catalogue's id, the version the host
/// states for it, and the file holding its entries, with the workbook sheet
/// to read.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CatalogueArg {
    catalogue: String,
    version: Option<String>,
    path: PathBuf,
    sheet: Option<String>,
}

/// Parses `CATALOGUE[@VERSION]=FILE[#SHEET]`: as a `--relations` argument,
/// the version, if any, after the first `@` of the part before the `=`.
fn catalogue_arg(value: &str) -> Result<CatalogueArg, String> {
    let refused = || "expected CATALOGUE[@VERSION]=FILE, each non-empty".to_owned();
    let RelationArg {
        relation,
        path,
        sheet,
    } = relation_arg(value).map_err(|_| refused())?;
    let (catalogue, version) = match relation.split_once('@') {
        Some((catalogue, version)) => (catalogue, Some(version)),
        None => (relation.as_str(), None),
    };
    if catalogue.is_empty() || version.is_some_and(|version| version.trim().is_empty()) {
        return Err(refused());
    }
    Ok(CatalogueArg {
        catalogue: catalogue.to_owned(),
        version: version.map(str::to_owned),
        path,
        sheet,
    })
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

/// The package file at `path`'s root: the directory holding it.
fn package_root(path: &Path) -> axioval::engine::PackageDirectory {
    axioval::engine::PackageDirectory::new(match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    })
}

/// A package file as given: an MCS archive, or the bytes of a JSON package.
enum PackageFile {
    Archive(Box<axioval_mcs::Archive>),
    Json(Vec<u8>),
}

/// The package file at `path`, read as an MCS archive when it is one: a
/// `.mcs` file, or one opening as a ZIP archive (`PK`), as an ifcZIP model
/// is recognised.
fn read_package(path: &Path) -> Result<PackageFile, Box<dyn Error>> {
    let shown = path.display();
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mcs"))
    {
        let archive = axioval_mcs::read_file(path).map_err(|error| format!("{shown}: {error}"))?;
        return Ok(PackageFile::Archive(Box::new(archive)));
    }
    let bytes = fs::read(path).map_err(|error| format!("{shown}: {error}"))?;
    if bytes.starts_with(b"PK\x03\x04") {
        let archive = axioval_mcs::read(&bytes).map_err(|error| format!("{shown}: {error}"))?;
        return Ok(PackageFile::Archive(Box::new(archive)));
    }
    Ok(PackageFile::Json(bytes))
}

/// The JSON package `bytes` read from `path`.
fn parse<T: serde::de::DeserializeOwned>(path: &Path, bytes: &[u8]) -> Result<T, Box<dyn Error>> {
    let text =
        std::str::from_utf8(bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(serde_json::from_str(text).map_err(|error| format!("{}: {error}", path.display()))?)
}

/// The definition packages at `path`: one JSON package, with the table
/// files its defaults name loaded from beside it, or the documents of an
/// MCS `definitions` archive, with theirs loaded from the archive.
pub(crate) fn load_definitions(path: &Path) -> Result<Vec<DefinitionPackage>, Box<dyn Error>> {
    match read_package(path)? {
        PackageFile::Archive(archive) => Ok(archive.into_definitions().map_err(|error| {
            format!(
                "{}: {error}; a rule-set archive is given with --ruleset, and carries its \
                 definitions",
                path.display()
            )
        })?),
        PackageFile::Json(bytes) => {
            let mut package: DefinitionPackage = parse(path, &bytes)?;
            axioval::engine::load_definition_table_files(&mut package, &package_root(path))
                .map_err(|error| format!("{}: {error}", path.display()))?;
            Ok(vec![package])
        }
    }
}

/// The ruleset at `path` and the definition packages it carries: a JSON
/// ruleset, with the table files its rules name loaded from beside it, and
/// none, or the rule set of an MCS `ruleset` archive and its definition
/// documents, with their table files loaded from the archive.
pub(crate) fn load_ruleset(
    path: &Path,
) -> Result<(RuleSetPackage, Vec<DefinitionPackage>), Box<dyn Error>> {
    match read_package(path)? {
        PackageFile::Archive(archive) => Ok(archive.into_ruleset().map_err(|error| {
            format!(
                "{}: {error}; a definitions archive is given with --definitions",
                path.display()
            )
        })?),
        PackageFile::Json(bytes) => {
            let mut ruleset: RuleSetPackage = parse(path, &bytes)?;
            axioval::engine::load_table_files(&mut ruleset, &package_root(path))
                .map_err(|error| format!("{}: {error}", path.display()))?;
            Ok((ruleset, Vec::new()))
        }
    }
}

/// The definition packages and rulesets of `--definitions` and
/// `--ruleset`: every definition package given, then each one a rule-set
/// archive carries that none given has the ID of
/// (`axioval_mcs::resolve_definitions`).
pub(crate) fn packages(
    definitions: &[PathBuf],
    rulesets: &[PathBuf],
) -> Result<(Vec<DefinitionPackage>, Vec<RuleSetPackage>), Box<dyn Error>> {
    let mut supplied = Vec::new();
    for path in definitions {
        supplied.extend(load_definitions(path)?);
    }
    let mut carried = Vec::new();
    let mut loaded = Vec::with_capacity(rulesets.len());
    for path in rulesets {
        let (ruleset, definitions) = load_ruleset(path)?;
        loaded.push(ruleset);
        carried.extend(definitions);
    }
    Ok((axioval_mcs::resolve_definitions(supplied, carried)?, loaded))
}

fn run() -> Result<Outcome, Box<dyn Error>> {
    match Cli::parse().command {
        Command::Validate(args) => draft::run(&args),
        Command::Catalogue {
            definitions,
            output,
            locale,
        } => {
            let mut packages = Vec::new();
            for path in &definitions {
                packages.extend(load_definitions(path)?);
            }
            let catalogue = axioval::catalogue(&packages).map_err(|error| error.to_string())?;
            let mut json = match locale {
                Some(locale) => {
                    let (value, fallbacks) =
                        axioval::engine::catalogue::localized(&catalogue, &locale)?;
                    for pointer in fallbacks {
                        eprintln!("not translated into {locale}, English used: {pointer}");
                    }
                    serde_json::to_string_pretty(&value)?
                }
                None => serde_json::to_string_pretty(&catalogue)?,
            };
            json.push('\n');
            match output {
                Some(path) => fs::write(&path, json)?,
                None => std::io::Write::write_all(&mut std::io::stdout(), json.as_bytes())?,
            }
            Ok(Outcome::Passed)
        }
        Command::Check(args) => check(*args),
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
        Command::Export(args) => Ok(if args.run()? {
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

/// The bridge's facts about the model data, as integrity warnings.
pub(crate) fn model_data_records(found: Vec<geometry::ModelData>) -> Vec<IntegrityRecord> {
    found
        .into_iter()
        .map(|note| IntegrityRecord {
            code: note.code.to_owned(),
            severity: "warning".to_owned(),
            message: note.message,
            locator: note.locator,
        })
        .collect()
}

/// What `--geometry` measured, as the result records it.
fn geometry_record(report: geometry::GeometryReport) -> digest::GeometryRecord {
    digest::GeometryRecord {
        exact: report.exact,
        tessellated: report.tessellated,
        no_body: report.no_body,
        exact_boundaries: report.exact_boundaries,
        composed: report.composed,
        unmeasured: report
            .unmeasured
            .into_iter()
            .map(|(object, reason)| digest::Unmeasured { object, reason })
            .collect(),
        openings_taken_as_applied: report
            .applied_openings
            .into_iter()
            .map(|(host, opening, reason)| digest::AppliedOpening {
                host,
                opening,
                reason,
            })
            .collect(),
        whole_openings: report
            .whole_openings
            .into_iter()
            .map(|(whole, opening, subtracted_from)| digest::WholeOpening {
                whole,
                opening,
                subtracted_from,
            })
            .collect(),
        local_origin: report.local_origin,
        declared_frame_sources: report.declared_frame_sources,
        superseded: report
            .superseded
            .into_iter()
            .map(|matched| digest::SupersededRecord {
                derived: matched.derived,
                original: matched.original,
                matched: matched.matched.as_str().to_owned(),
            })
            .collect(),
        derived_unmatched: report
            .derived_unmatched
            .into_iter()
            .map(|(cause, count)| (cause.as_str().to_owned(), count))
            .collect(),
        not_meshed: report.not_meshed,
    }
}

/// How `check --geometry` meshes the models of `session`.
///
/// # Errors
///
/// A `--derived-from` that is not `DERIVED=ORIGINAL` or names a model the
/// check does not read.
fn geometry_options(
    args: &CheckArgs,
    session: &EvidenceSession,
) -> Result<geometry::Options, Box<dyn Error>> {
    let mut options = geometry::Options::meshes(args.output.bcf_view.snapshots)
        .with_exact_boundaries(!args.no_exact_boundaries)
        .with_declared_shared_frame(args.declare_shared_frame)
        .with_timings(args.timings)
        .with_derived_matching(match args.derived_match {
            DerivedMatch::Id => geometry::Matching::ById,
            DerivedMatch::IdOrGeometry => geometry::Matching::ByIdOrGeometry,
        })
        .with_derived_tolerance(args.derived_tolerance)
        .with_reuse(!args.no_reuse);
    let documents: Vec<&SourceId> = session
        .snapshots()
        .map(axioval::engine::SourceSnapshot::source)
        .collect();
    for argument in &args.derived_from {
        let source = |name: &str| {
            documents
                .iter()
                .find(|source| source.document == name)
                .map(|source| (*source).clone())
        };
        let named = argument
            .split_once('=')
            .and_then(|(derived, original)| Some((source(derived)?, source(original)?)));
        let Some((derived, original)) = named else {
            return Err(format!(
                "--derived-from {argument}: name two models as DERIVED=ORIGINAL by file name, one of {}",
                documents
                    .iter()
                    .map(|source| source.document.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .into());
        };
        options = options.with_derived_from(derived, original);
    }
    Ok(options)
}

fn check(mut args: CheckArgs) -> Result<Outcome, Box<dyn Error>> {
    let mut clock = timings::Stopwatch::new(args.timings);
    args.output.prepare()?;
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
    let mut plan =
        compile_rulesets(&registry, &definitions, &rulesets)?.evaluate_in(args.rule_order);
    let mut supplied = supply_inputs(&mut plan, &args)?;
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
    clock.lap("compile");
    let (session, bytes) = sources(&args.models)?;
    clock.lap("read models");
    let session = with_disciplines(session, &args);
    // Reads no body, so it stands whatever is meshed.
    let stale = results::supply_revisions(&mut plan, &args, &session)?;
    let unknown_pairs = axioval::engine::unknown_relation_objects(&plan, session.project());
    let demand = demand(&args, &plan, &registry, &session);
    let budget = memory_budget(args.memory_budget.as_deref())?;
    let terrain = supplied.terrain.take();
    let runtime = check_runtime(registry, &args, &clock, (terrain, budget.as_ref()))?;
    let ((session, mut meshed, mut result), demand) = checked(
        &args,
        (&runtime, &mut clock),
        &plan,
        (session, &bytes),
        demand,
    )?;
    clock.memory(budget.as_deref());
    // The zones are placed against every body of the session as checked.
    let placements = zones::check_placements(&session, &plan, &mut supplied.zone_files);
    supplied.unsupplied.extend(placements);
    supplied.unsupplied.extend(stale);
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
    let mut integrity = integrity(&session)?;
    integrity.extend(relation_records(unknown_pairs));
    integrity.append(&mut supplied.unsupplied);
    // Model data the bridge found (no shape at all, a face whose boundary
    // crosses itself), once per object.
    if let Some(report) = meshed.as_mut() {
        integrity.extend(model_data_records(std::mem::take(&mut report.model_data)));
    }

    let bodies = meshed
        .as_mut()
        .map(|report| snapshot_meshes(std::mem::take(&mut report.meshes)));
    let geometry = meshed.map(geometry_record);
    let output = CheckOutput::new(result, integrity, geometry, session.project())
        .with_sources(source_infos(&session))
        .with_unmatched_topics(unmatched);
    let mut output = supplied.recorded(output);
    let complete = translated.as_ref().is_none_or(ids::Translated::is_complete);
    if let Some(translated) = translated {
        output = output.with_ids(translated.record);
    }
    let bounds = args
        .geometry
        .then(|| match demand {
            GeometryDemand::Everything => Ok(bounds::bounds(&[&session], &output.report)),
            // Only a BCF archive frames objects, and a finding may name one
            // the rules never measured.
            _ if args.output.bcf.is_none() => Ok(BTreeMap::new()),
            _ => named_bounds(&args, &output.report),
        })
        .transpose()?;
    emit(
        &output,
        session.project(),
        (bounds, bodies),
        labels,
        args.output,
    )?;
    clock.finish("outputs");
    Ok(match Outcome::of(&output.report) {
        // A specification that did not run was not checked: never a pass.
        Outcome::Passed if !complete => Outcome::Incomplete,
        outcome => outcome,
    })
}

/// `session` with the disciplines `--discipline-map` declares.
fn with_disciplines(session: EvidenceSession, args: &CheckArgs) -> EvidenceSession {
    session.with_discipline_map(
        &args
            .discipline_map
            .iter()
            .cloned()
            .fold(DisciplineMap::new(), DisciplineMap::with),
    )
}

/// What `check --geometry` meshes for `plan` over `session` (#410): every
/// body when asked (`--mesh-all`), when spaces are located from bodies,
/// when a derived model is compared with its original, when BCF
/// snapshots draw the model or when context zones are placed against the
/// model's extent; otherwise only what the plan's rules can ask
/// about ([`axioval::engine::geometry_demand`]).
fn demand(
    args: &CheckArgs,
    plan: &ExecutionPlan,
    registry: &axioval::engine::CapabilityRegistry,
    session: &EvidenceSession,
) -> GeometryDemand {
    if args.mesh_all
        || args.locate == Locate::Geometry
        || !args.derived_from.is_empty()
        || args.output.bcf_view.snapshots
        || !plan.context_zones().files().is_empty()
    {
        return GeometryDemand::Everything;
    }
    axioval::engine::geometry_demand(plan, registry, session)
}

/// What a kept run of `check` checked: the session, what was meshed and
/// the report.
type Kept = (EvidenceSession, Option<geometry::GeometryReport>, Report);

/// A run of `check`: kept, or reaching geometry it had left out.
enum Checked {
    /// The run, kept.
    Kept(Box<Kept>),
    /// A rule asked for a service or a body left out, as named: the report
    /// cannot be kept.
    Reached(String),
}

/// [`run_check`] meshing what `demand` asks for, and again with every body
/// meshed from freshly read sources when a rule reached geometry it had
/// left out: the answers assumed an object absent that was only left
/// unmeshed, so they are never kept and the report is the same as ever.
/// Returns the kept run and the demand it meshed for.
fn checked(
    args: &CheckArgs,
    (runtime, clock): (&Runtime, &mut timings::Stopwatch),
    plan: &ExecutionPlan,
    (session, bytes): (EvidenceSession, &geometry::ModelBytes),
    demand: GeometryDemand,
) -> Result<(Kept, GeometryDemand), Box<dyn Error>> {
    let why = match run_check(args, (runtime, clock), plan, (session, bytes), &demand)? {
        Checked::Kept(kept) => return Ok((*kept, demand)),
        Checked::Reached(why) => why,
    };
    eprintln!(
        "note: a rule asked for geometry the check had left out ({why}); \
         meshing every object and checking again"
    );
    let (session, bytes) = sources(&args.models)?;
    let session = with_disciplines(session, args);
    let every = GeometryDemand::Everything;
    match run_check(args, (runtime, clock), plan, (session, &bytes), &every)? {
        Checked::Kept(kept) => Ok((*kept, every)),
        Checked::Reached(why) => Err(format!("geometry: every object meshed, yet {why}").into()),
    }
}

/// Meshes what `demand` asks for (with `--geometry`) and runs `plan` over
/// the session. With less than every body meshed, the session watches the
/// geometry services left out, or every service when nothing is meshed,
/// and the bodies left out; a rule reaching either makes the run
/// [`Checked::Reached`].
fn run_check(
    args: &CheckArgs,
    (runtime, clock): (&Runtime, &mut timings::Stopwatch),
    plan: &ExecutionPlan,
    (session, bytes): (EvidenceSession, &geometry::ModelBytes),
    demand: &GeometryDemand,
) -> Result<Checked, Box<dyn Error>> {
    if !args.geometry {
        let report = runtime.run_session(&session, plan.clone())?;
        clock.lap("run (all rules)");
        return Ok(Checked::Kept(Box::new((session, None, report))));
    }
    let every = geometry::service_types();
    let (session, report, watched) = match demand {
        GeometryDemand::Nothing => {
            // Nothing attached: every geometry service, and the local
            // frames, are left out.
            let report =
                geometry::unattached(&session).map_err(|error| format!("geometry: {error}"))?;
            (session, report, Some(every))
        }
        demand => {
            let mut options = geometry_options(args, &session)?;
            let mut watched = None;
            if let GeometryDemand::Objects { objects, services } = demand {
                options = options.with_only(objects.clone());
                // The local frames are the same whatever is meshed.
                let frames = std::any::TypeId::of::<axioval::engine::LocalOrigins>();
                watched = Some(
                    every
                        .into_iter()
                        .filter(|(id, _)| !services.contains(id) && *id != frames)
                        .collect(),
                );
            }
            let (session, report) = geometry::attach(session, bytes, options)
                .map_err(|error| format!("geometry: {error}"))?;
            (session, report, watched)
        }
    };
    // Each geometry phase is timed by `attach`.
    clock.skip();
    let (session, watch) = match watched {
        Some(types) => {
            let (session, watch) = session.watching(types);
            (session, Some(watch))
        }
        None => (session, None),
    };
    let result = runtime.run_session(&session, plan.clone())?;
    clock.lap("run (all rules)");
    let asked = watch.map(|watch| watch.asked()).unwrap_or_default();
    if !asked.is_empty() {
        return Ok(Checked::Reached(asked.join(", ")));
    }
    if report.reached_left_out() {
        return Ok(Checked::Reached(
            "a body no rule was expected to ask about".to_owned(),
        ));
    }
    Ok(Checked::Kept(Box::new((session, Some(report), result))))
}

/// The measured extent of every object `report` names, meshed for that
/// alone: a run that meshed only what its rules asked about may name
/// others in its findings, and BCF cameras frame them all. Each object is
/// meshed as among all the others ([`geometry::Options::with_only`]), so
/// its extent is the one a run meshing everything measures.
fn named_bounds(
    args: &CheckArgs,
    report: &Report,
) -> Result<BTreeMap<ObjectId, bcf::Bounds>, Box<dyn Error>> {
    let mut named: BTreeSet<ObjectId> = BTreeSet::new();
    for finding in report.findings() {
        named.extend(finding.object_id().cloned());
        named.extend(finding.related.iter().cloned());
    }
    for outcome in report.not_evaluated() {
        named.extend(outcome.object_id().cloned());
    }
    if named.is_empty() {
        return Ok(BTreeMap::new());
    }
    let (session, bytes) = sources(&args.models)?;
    let session = with_disciplines(session, args);
    let options = geometry_options(args, &session)?.with_only(named);
    let (session, _) =
        geometry::attach(session, &bytes, options).map_err(|error| format!("geometry: {error}"))?;
    Ok(bounds::bounds(&[&session], report))
}

/// What `check` supplied beside the model before anything ran: the files
/// of relation pairs, catalogue entries and context zones, and a warning
/// for each input the plan reads that no file supplied.
struct Supplied {
    relation_files: Vec<digest::RelationFileRecord>,
    catalogue_files: Vec<digest::CatalogueFileRecord>,
    zone_files: Vec<digest::ZoneFileRecord>,
    terrain_files: Vec<digest::TerrainFileRecord>,
    /// The terrain the run measures against, `None` without any.
    terrain: Option<axioval::engine::TerrainServiceHandle>,
    unsupplied: Vec<IntegrityRecord>,
}

impl Supplied {
    /// `output` recording every file supplied.
    fn recorded(self, output: CheckOutput) -> CheckOutput {
        output
            .with_relation_files(self.relation_files)
            .with_catalogue_files(self.catalogue_files)
            .with_zone_files(self.zone_files)
            .with_terrain_files(self.terrain_files)
    }
}

/// Supplies `plan` with the relation pairs, catalogue entries and context
/// zones `args` name, and reads the terrain surfaces, before any model is
/// read.
///
/// # Errors
///
/// Any file refused (see [`supply_relations`], [`supply_catalogues`] and
/// [`zones::supply`]), and any terrain file refused
/// ([`terrain::read_terrains`]).
fn supply_inputs(plan: &mut ExecutionPlan, args: &CheckArgs) -> Result<Supplied, Box<dyn Error>> {
    let relation_files = supply_relations(plan, &args.relations)?;
    let catalogue_files = supply_catalogues(plan, &args.catalogues)?;
    let (zone_files, unsupplied_zones) = zones::supply(plan, &args.zones)?;
    let mut unsupplied = unsupplied_relations(plan);
    unsupplied.extend(unsupplied_catalogues(plan));
    unsupplied.extend(unsupplied_zones);
    let terrain = terrain::read_terrains(&args.terrains, args.terrain_in, plan)?;
    unsupplied.extend(terrain.warnings);
    Ok(Supplied {
        relation_files,
        catalogue_files,
        zone_files,
        terrain_files: terrain.files,
        terrain: terrain.service,
        unsupplied,
    })
}

/// `--rule-order`: `plan`, `reversed` or `shuffled:SEED`.
fn rule_order(text: &str) -> Result<axioval::engine::RuleOrder, String> {
    use axioval::engine::RuleOrder;
    match text {
        "plan" => Ok(RuleOrder::Plan),
        "reversed" => Ok(RuleOrder::Reversed),
        _ => text
            .strip_prefix("shuffled:")
            .and_then(|seed| seed.parse().ok())
            .map(RuleOrder::Shuffled)
            .ok_or_else(|| format!("`{text}` is not `plan`, `reversed` or `shuffled:SEED`")),
    }
}

/// The environment variable naming how many threads `check` evaluates a
/// rule's subjects on when `--threads` does not.
const THREADS_VARIABLE: &str = "AXIOVAL_THREADS";

/// The most threads `check` uses when neither `--threads` nor
/// [`THREADS_VARIABLE`] names a count. Four keep most of the speed-up;
/// the heavy steps' memory is bounded by the memory budget (#407), but
/// each thread still holds its subject's scene, so more threads are asked
/// for, not taken.
const DEFAULT_THREADS: std::num::NonZeroUsize = std::num::NonZeroUsize::new(4).unwrap();

/// How many threads `check` evaluates each rule's subjects on: `--threads`,
/// else [`THREADS_VARIABLE`], else the machine's available parallelism up
/// to [`DEFAULT_THREADS`].
///
/// # Errors
///
/// [`THREADS_VARIABLE`] set to anything but a positive whole number.
fn parallelism(
    threads: Option<std::num::NonZeroUsize>,
) -> Result<axioval::engine::Parallelism, Box<dyn Error>> {
    use axioval::engine::Parallelism;
    if let Some(threads) = threads {
        return Ok(Parallelism::new(threads));
    }
    match std::env::var(THREADS_VARIABLE) {
        Ok(value) => value.trim().parse().map(Parallelism::new).map_err(|_| {
            format!("{THREADS_VARIABLE}={value} is not a positive whole number of threads").into()
        }),
        Err(std::env::VarError::NotPresent) => Ok(Parallelism::new(
            std::thread::available_parallelism()
                .map_or(DEFAULT_THREADS, |available| available.min(DEFAULT_THREADS)),
        )),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(format!("{THREADS_VARIABLE} is not a positive whole number of threads").into())
        }
    }
}

/// The environment variable naming `check`'s memory budget when
/// `--memory-budget` does not.
const MEMORY_BUDGET_VARIABLE: &str = "AXIOVAL_MEMORY_BUDGET";

/// The memory budget `check` admits its heavy steps against:
/// `--memory-budget`, else [`MEMORY_BUDGET_VARIABLE`], else half the
/// memory the system reports available, at least
/// [`MemoryBudget::FLOOR`](axioval::engine::MemoryBudget::FLOOR); `none`
/// for no budget, as where the system does not say.
///
/// # Errors
///
/// A stated budget that is not a size or `none`.
fn memory_budget(
    stated: Option<&str>,
) -> Result<Option<std::sync::Arc<axioval::engine::MemoryBudget>>, Box<dyn Error>> {
    use axioval::engine::{MemoryBudget, memory};
    let stated = match stated {
        Some(stated) => Some((stated.to_owned(), "--memory-budget")),
        None => match std::env::var(MEMORY_BUDGET_VARIABLE) {
            Ok(value) => Some((value, MEMORY_BUDGET_VARIABLE)),
            Err(std::env::VarError::NotPresent) => None,
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err(format!("{MEMORY_BUDGET_VARIABLE} is not a size").into());
            }
        },
    };
    let budget = match stated {
        Some((text, _)) if text.trim().eq_ignore_ascii_case("none") => None,
        Some((text, name)) => Some(MemoryBudget::new(
            memory::parse_size(&text).map_err(|error| format!("{name}: {error}"))?,
        )),
        None => MemoryBudget::from_available(),
    };
    Ok(budget.map(std::sync::Arc::new))
}

/// The runtime `check` runs its plan with: located, summarizing and timing
/// rules, on as many threads and reusing as much, as its arguments ask,
/// against the terrain and memory budget given.
///
/// # Errors
///
/// [`THREADS_VARIABLE`] set to anything but a positive whole number.
fn check_runtime(
    registry: axioval::engine::CapabilityRegistry,
    args: &CheckArgs,
    clock: &timings::Stopwatch,
    (terrain, budget): (
        Option<axioval::engine::TerrainServiceHandle>,
        Option<&std::sync::Arc<axioval::engine::MemoryBudget>>,
    ),
) -> Result<Runtime, Box<dyn Error>> {
    let mut runtime = Runtime::new(registry).with_parallelism(parallelism(args.threads)?);
    if args.no_reuse {
        runtime = runtime.without_reuse();
    }
    if let Some(bytes) = args.keep_derivations {
        runtime = runtime.with_kept_derivations(bytes);
    }
    if let Some(nodes) = args.evaluation_budget {
        runtime = runtime.with_evaluation_budget(nodes);
    }
    if let Some(budget) = budget {
        runtime = runtime.with_memory_budget(budget.clone());
    }
    if let Some(terrain) = terrain {
        runtime = runtime.with_terrain(terrain);
    }
    if let Some(policy) = args.locate.policy() {
        runtime = runtime.with_locations(policy);
    }
    if args.rule_status {
        runtime = runtime.with_rule_summaries();
    }
    if args.rule_timings {
        runtime = runtime.with_rule_timings();
    }
    if let Some(observer) = clock.rules() {
        runtime = runtime.with_rule_observer(observer);
    }
    if args.stats {
        runtime = runtime.with_derivation_observer(stats::observer());
    }
    Ok(runtime)
}

/// Reads each `--relations` file by its relation's declared columns and
/// supplies its pairs to `plan`, before anything runs.
///
/// # Errors
///
/// A relation no ruleset declares, one that states its own pairs, a
/// relation given two files, and a file that cannot be read, is too large,
/// or whose columns or rows are refused.
fn supply_relations(
    plan: &mut ExecutionPlan,
    args: &[RelationArg],
) -> Result<Vec<digest::RelationFileRecord>, Box<dyn Error>> {
    let mut records = Vec::new();
    for arg in args {
        let shown = arg.path.display();
        let refuse = |detail: String| format!("--relations {}={shown}: {detail}", arg.relation);
        let definition = plan
            .relations()
            .iter()
            .find(|known| known.id == arg.relation)
            .cloned()
            .ok_or_else(|| {
                refuse(format!(
                    "no ruleset declares the relation `{}`",
                    arg.relation
                ))
            })?;
        let size = fs::metadata(&arg.path)
            .map_err(|error| refuse(error.to_string()))?
            .len();
        if size > axioval::engine::TABLE_FILE_LIMIT_BYTES {
            return Err(refuse(format!(
                "the file is larger than {} bytes",
                axioval::engine::TABLE_FILE_LIMIT_BYTES
            ))
            .into());
        }
        let bytes = fs::read(&arg.path).map_err(|error| refuse(error.to_string()))?;
        let name = arg
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| refuse("the file name is not UTF-8".into()))?;
        let pairs =
            SuppliedPairs::read(&definition, &bytes, name, arg.sheet.as_deref()).map_err(refuse)?;
        records.push(digest::RelationFileRecord {
            relation: arg.relation.clone(),
            file: pairs.origin().to_owned(),
            sha256: pairs.sha256().to_owned(),
            pairs: pairs.rows().len(),
        });
        plan.supply_relation(&arg.relation, pairs)
            .map_err(|error| refuse(error.to_string()))?;
    }
    Ok(records)
}

/// Reads each `--catalogue` file by its catalogue's declared columns and
/// supplies its entries to `plan`, with the version the argument states,
/// before anything runs.
///
/// # Errors
///
/// A catalogue no ruleset declares, a catalogue given two files, and a
/// file that cannot be read, is too large, or whose columns or entries are
/// refused.
fn supply_catalogues(
    plan: &mut ExecutionPlan,
    args: &[CatalogueArg],
) -> Result<Vec<digest::CatalogueFileRecord>, Box<dyn Error>> {
    let mut records = Vec::new();
    for arg in args {
        let shown = arg.path.display();
        let refuse = |detail: String| format!("--catalogue {}={shown}: {detail}", arg.catalogue);
        let definition = plan
            .catalogues()
            .into_iter()
            .find(|known| known.id == arg.catalogue)
            .cloned()
            .ok_or_else(|| {
                refuse(format!(
                    "no ruleset declares the catalogue `{}`",
                    arg.catalogue
                ))
            })?;
        let size = fs::metadata(&arg.path)
            .map_err(|error| refuse(error.to_string()))?
            .len();
        if size > axioval::engine::TABLE_FILE_LIMIT_BYTES {
            return Err(refuse(format!(
                "the file is larger than {} bytes",
                axioval::engine::TABLE_FILE_LIMIT_BYTES
            ))
            .into());
        }
        let bytes = fs::read(&arg.path).map_err(|error| refuse(error.to_string()))?;
        let name = arg
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| refuse("the file name is not UTF-8".into()))?;
        let catalogue = SuppliedCatalogue::read(
            &definition,
            &bytes,
            name,
            arg.sheet.as_deref(),
            arg.version.as_deref(),
        )
        .map_err(refuse)?;
        records.push(digest::CatalogueFileRecord {
            catalogue: arg.catalogue.clone(),
            version: arg.version.clone(),
            file: catalogue.origin().to_owned(),
            sha256: catalogue.sha256().to_owned(),
            entries: catalogue.rows().len(),
        });
        plan.supply_catalogue(&arg.catalogue, catalogue)
            .map_err(|error| refuse(error.to_string()))?;
    }
    Ok(records)
}

/// Every catalogue given no file, as an integrity warning: the objects
/// whose expressions read it are not evaluated.
fn unsupplied_catalogues(plan: &ExecutionPlan) -> Vec<IntegrityRecord> {
    plan.catalogues()
        .into_iter()
        .filter(|catalogue| plan.supplied_catalogue(&catalogue.id).is_none())
        .map(|catalogue| IntegrityRecord {
            code: "catalogue-not-supplied".into(),
            severity: "warning".into(),
            message: format!(
                "catalogue `{}` takes its entries at check time, and none were supplied \
                 (`--catalogue {}=FILE`); every object whose expression reads it is not \
                 evaluated",
                catalogue.id, catalogue.id
            ),
            locator: format!(
                "{};id={}",
                axioval::engine::CATALOGUE_LOCATOR_PREFIX,
                catalogue.id
            ),
        })
        .collect()
}

/// Every supplied relation given no file, as an integrity warning: the
/// rules walking it are not evaluated.
fn unsupplied_relations(plan: &ExecutionPlan) -> Vec<IntegrityRecord> {
    plan.relations()
        .iter()
        .filter(|relation| {
            matches!(
                relation.by,
                axioval::ir::contract::RelationKey::Supplied { .. }
            ) && plan.supplied_pairs(&relation.id).is_none()
        })
        .map(|relation| IntegrityRecord {
            code: "relation-pairs-not-supplied".into(),
            severity: "warning".into(),
            message: format!(
                "relation `{}` takes its pairs at check time, and none were supplied \
                 (`--relations {}=FILE`); every rule walking it is not evaluated",
                relation.id, relation.id
            ),
            locator: format!(
                "{}{}:pairs",
                axioval::engine::RELATION_RELATIONSHIP_PREFIX,
                relation.id
            ),
        })
        .collect()
}

/// Every end of a declared relation's listed pair naming no single object,
/// as an integrity warning: reported, never dropped. The rules walking its
/// relation leave the pair undecided.
fn relation_records(unknown: Vec<axioval::engine::UnknownRelationObject>) -> Vec<IntegrityRecord> {
    unknown
        .into_iter()
        .map(|unknown| IntegrityRecord {
            code: "relation-object-unknown".into(),
            severity: "warning".into(),
            message: format!(
                "relation `{}` row {} `{}`: {}",
                unknown.relation, unknown.row, unknown.end, unknown.detail
            ),
            locator: format!(
                "{}{}:pairs#row={}",
                axioval::engine::RELATION_RELATIONSHIP_PREFIX,
                unknown.relation,
                unknown.row
            ),
        })
        .collect()
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

/// Each meshed object's triangles, to draw BCF snapshots from.
pub(crate) type Meshes = BTreeMap<ObjectId, bcf_snapshot::Mesh>;

/// The meshes the bridge kept, as the snapshot renderer takes them.
pub(crate) fn snapshot_meshes(kept: BTreeMap<ObjectId, geometry::KeptMesh>) -> Meshes {
    kept.into_iter()
        .filter_map(|(object, mesh)| {
            let (positions, triangles) = mesh.into_parts();
            Some((object, bcf_snapshot::Mesh::new(positions, triangles)?))
        })
        .collect()
}

/// Writes a result as `args` asks: JSON to stdout or `--report`, a summary,
/// a BCF archive, and diagnostics to stderr. `bounds` are the measured
/// objects' extents with `--geometry`, which fit the BCF cameras;
/// `rule_labels` label each rule's topics (see [`rule_labels`]).
///
/// Everything is built before anything is written, so a run that fails
/// leaves no partial output behind.
pub(crate) fn emit(
    output: &CheckOutput,
    project: &Project,
    (bounds, meshes): (Option<BTreeMap<ObjectId, bcf::Bounds>>, Option<Meshes>),
    rule_labels: BTreeMap<String, Vec<String>>,
    args: OutputArgs,
) -> Result<(), Box<dyn Error>> {
    let json = serde_json::to_string_pretty(output)? + "\n";
    let documents = documents(output, project, &args)?;
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
                frame_witness: args.bcf_view.frame_witness,
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
    for (path, bytes) in &documents {
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

/// A document to write: its path and bytes.
type Document = (PathBuf, Vec<u8>);

/// The `--xlsx` workbook and the `--html` report, each with its path, built
/// but not written.
fn documents(
    output: &CheckOutput,
    project: &Project,
    args: &OutputArgs,
) -> Result<Vec<Document>, Box<dyn Error>> {
    let mut documents = Vec::new();
    if let Some(path) = &args.xlsx {
        let options = xlsx::Options {
            external_id_scheme: Some(bcf::IFC_GLOBAL_ID_SCHEME.to_owned()),
            ..xlsx::Options::new(i64::try_from(epoch_seconds()?)?)
        };
        let bytes = xlsx::export(&output.report, project, &options)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        documents.push((path.clone(), bytes));
    }
    if let Some(path) = &args.html {
        let options = html::Options {
            title: args.html_title.clone(),
            external_id_scheme: Some(bcf::IFC_GLOBAL_ID_SCHEME.to_owned()),
            ..html::Options::new(timestamp()?)
        };
        let template = args.template.clone().unwrap_or_default();
        let text = html::render(&output.report, project, &template, &options);
        documents.push((path.clone(), text.into_bytes()));
    }
    Ok(documents)
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
                "geometry: {} exact, {} tessellated, {} without body, {} unmeasured, \
                 {} with an exact boundary{}{}{}{}",
                geometry.exact,
                geometry.tessellated,
                geometry.no_body,
                geometry.unmeasured.len(),
                geometry.exact_boundaries,
                if geometry.not_meshed == 0 {
                    String::new()
                } else {
                    format!(
                        ", {} not meshed (no rule asks about them)",
                        geometry.not_meshed
                    )
                },
                if geometry.composed == 0 {
                    String::new()
                } else {
                    format!(", {} measured through their parts", geometry.composed)
                },
                if geometry.openings_taken_as_applied.is_empty() {
                    String::new()
                } else {
                    format!(
                        ", {} opening(s) taken as already applied",
                        geometry.openings_taken_as_applied.len()
                    )
                },
                match geometry
                    .whole_openings
                    .iter()
                    .filter(|opening| !opening.subtracted_from.is_empty())
                    .count()
                {
                    0 => String::new(),
                    count => format!(", {count} opening(s) of wholes subtracted from their parts"),
                }
            );
            for source in &geometry.declared_frame_sources {
                eprintln!(
                    "note: {source} is measured in the first model's frame as declared \
                     (--declare-shared-frame), not as its coordinate system states"
                );
            }
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
    let session = if ifc::is_ifc_xml(&content) {
        ifc::import_ifc_xml_session(document, &content)
    } else {
        ifc::import_ifc_session(document, &content)
    }
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
    use super::{CatalogueArg, ModelArg, RelationArg, catalogue_arg, model_arg, relation_arg, utc};
    use axioval::ir::Discipline;
    use std::path::PathBuf;

    #[test]
    fn a_relations_argument_names_a_relation_a_file_and_a_workbook_sheet() {
        let parsed = |relation: &str, path: &str, sheet: Option<&str>| RelationArg {
            relation: relation.into(),
            path: PathBuf::from(path),
            sheet: sheet.map(Into::into),
        };
        assert_eq!(
            relation_arg("serves=dir/pairs.csv"),
            Ok(parsed("serves", "dir/pairs.csv", None))
        );
        assert_eq!(
            relation_arg("serves=p.xlsx#Pumps"),
            Ok(parsed("serves", "p.xlsx", Some("Pumps")))
        );
        assert_eq!(
            relation_arg("serves=a#b.csv"),
            Ok(parsed("serves", "a#b.csv", None))
        );
        assert!(relation_arg("serves").is_err());
        assert!(relation_arg("=p.csv").is_err());
        assert!(relation_arg("serves=").is_err());
    }

    #[test]
    fn a_catalogue_argument_names_a_catalogue_its_version_and_a_file() {
        let parsed = |catalogue: &str, version: Option<&str>, path: &str, sheet: Option<&str>| {
            CatalogueArg {
                catalogue: catalogue.into(),
                version: version.map(Into::into),
                path: PathBuf::from(path),
                sheet: sheet.map(Into::into),
            }
        };
        assert_eq!(
            catalogue_arg("materials=m.csv"),
            Ok(parsed("materials", None, "m.csv", None))
        );
        assert_eq!(
            catalogue_arg("materials@2026-1=dir/m.xlsx#Data"),
            Ok(parsed(
                "materials",
                Some("2026-1"),
                "dir/m.xlsx",
                Some("Data")
            ))
        );
        assert!(catalogue_arg("materials").is_err());
        assert!(catalogue_arg("@2026=m.csv").is_err());
        assert!(catalogue_arg("materials@=m.csv").is_err());
        assert!(catalogue_arg("materials=").is_err());
    }

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
