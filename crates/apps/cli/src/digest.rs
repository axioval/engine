#![allow(clippy::doc_markdown)]

//! The saved check result, and bounded views of it.
//!
//! A full result grows with the model: one entry per finding, and a real model
//! easily has thousands. A reader with a budget (a person at a terminal, an
//! agent paying per token) needs the shape first and the entries only on
//! request. So there are two views, both computed from the saved result
//! without re-running the check:
//!
//! - a **summary**, whose size depends on how many distinct rules and codes
//!   fired, not on how many objects they fired on;
//! - a **listing**, filtered and paged, for digging into one rule, code or
//!   object.
//!
//! Both end with the exact command for the next step.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use axioval::ir::{
    Decision, DecisionComment, DecisionStatus, EvidenceCheck, Finding, FindingDecision, Location,
    NotEvaluated, NotEvaluatedReason, ObjectId, Place, Project, Report, ReportColumn,
    ReportColumnKind, ReportRow, ReportTable, ReportValue, RuleStatus, Scope, Severity, SourceId,
};
use serde::{Deserialize, Serialize};

/// Scheme of the GlobalId alias shown next to an object.
const GLOBAL_ID: &str = axioval::bcf::IFC_GLOBAL_ID_SCHEME;
/// Longest message a summary prints before eliding the rest.
const MESSAGE_BUDGET: usize = 160;
/// Example objects a summary names per group.
const EXAMPLES: usize = 3;

/// The JSON document `check` writes.
#[derive(Debug, Serialize, Deserialize)]
pub struct CheckOutput {
    pub report: Report,
    /// Irregularities of the model itself, separate from rule findings.
    pub integrity: Vec<IntegrityRecord>,
    /// Every object the report names, keyed by its full id, so a reader can
    /// tell what `#4711` is without the model at hand.
    #[serde(default)]
    pub objects: BTreeMap<String, ObjectInfo>,
    /// How the model's bodies were meshed, when `check --geometry` ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<GeometryRecord>,
    /// What `compare` found, per object and facet, beside the report it
    /// projects to. Absent from a `check` result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison: Option<ComparisonRecord>,
    /// Every checked source with its discipline and where it came from.
    /// Absent from a `compare` result and from results saved before it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceInfo>,
    /// What became of each specification of the IDS document `check --ids`
    /// ran. Absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ids: Option<IdsRecord>,
    /// The BCF topics `check --decisions-from` read that decided no
    /// current finding, in archive order. Absent otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unmatched_topics: Vec<UnmatchedTopicRecord>,
}

/// A BCF topic that decided no current finding, and why.
#[derive(Debug, Serialize, Deserialize)]
pub struct UnmatchedTopicRecord {
    /// The topic GUID as written, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// `no-finding`, `not-evaluated`, `no-guid` or `unreadable`.
    pub reason: String,
    /// What could not be read, for `unreadable`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl From<axioval::bcf::UnmatchedTopic> for UnmatchedTopicRecord {
    fn from(topic: axioval::bcf::UnmatchedTopic) -> Self {
        Self {
            guid: topic.guid,
            title: topic.title,
            status: topic.status,
            reason: topic.reason.as_str().to_owned(),
            detail: match topic.reason {
                axioval::bcf::Unmatched::Unreadable(why) => Some(why),
                _ => None,
            },
        }
    }
}

/// The IDS document `check --ids` translated and ran.
#[derive(Debug, Serialize, Deserialize)]
pub struct IdsRecord {
    /// The document's file name.
    pub document: String,
    /// The `--ids-filter` prefilter every specification was restricted by,
    /// as given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<axioval::ir::contract::Selector>,
    /// Every specification, in document order.
    pub specifications: Vec<IdsSpecification>,
}

/// One specification of an IDS document: the rules it ran as, or why it
/// ran none.
#[derive(Debug, Serialize, Deserialize)]
pub struct IdsSpecification {
    /// Position in the document, from 1.
    pub number: usize,
    /// `@name`.
    pub name: String,
    /// Ids of the rules it ran as; none when it has gaps.
    pub rules: Vec<String>,
    /// What could not be translated exactly, each as `part: reason`. A
    /// specification with any gap runs none of its rules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<String>,
}

/// One checked source: its discipline, declared or assigned by
/// `--discipline-map`, or why the map left it without one.
#[derive(Debug, Serialize, Deserialize)]
pub struct SourceInfo {
    /// `system:document`.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discipline: Option<String>,
    /// `declared` or `mapped`, with a discipline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discipline_origin: Option<String>,
    /// The map rule that assigned it, `field:pattern=discipline`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mapped_by: Option<String>,
    /// The value the rule matched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mapped_value: Option<String>,
    /// Why `--discipline-map` assigned none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unmapped: Option<String>,
}

/// The structured result of `compare`: every identity that is not unchanged,
/// with its differences per facet, and everything that could not be matched.
#[derive(Debug, Serialize, Deserialize)]
pub struct ComparisonRecord {
    /// The base source, `system:document`.
    pub base: String,
    /// The revised source.
    pub revised: String,
    /// The identity scheme objects were matched by.
    pub scheme: String,
    /// The facets compared, in order.
    pub facets: Vec<String>,
    pub tolerance: ToleranceRecord,
    pub counts: ComparisonCounts,
    /// Added, removed, changed and incomplete identities, by identity.
    /// Unchanged identities are only counted.
    pub objects: Vec<ComparedRecord>,
    /// Objects without an identity in the scheme.
    pub unidentified: Vec<SideObject>,
    /// Identities claimed by several objects of one side.
    pub ambiguous: Vec<AmbiguousRecord>,
    /// Coordinate systems, per pair of sources.
    pub coordinate_systems: Vec<SourceRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToleranceRecord {
    pub length_metres: f64,
    pub angle_degrees: f64,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct ComparisonCounts {
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
    pub unchanged: usize,
    /// Matched without a difference, but with a facet not compared or a
    /// measure undetermined.
    pub incomplete: usize,
    pub unidentified: usize,
    pub ambiguous: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ComparedRecord {
    pub identity: String,
    /// `added`, `removed`, `changed` or `incomplete`.
    pub state: String,
    /// The object's kind, from the revised side when it has one.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revised: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<ChangeRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved: Vec<GapRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub undetermined: Vec<ChangeRecord>,
}

/// One difference, or one undetermined measure.
#[derive(Debug, Serialize, Deserialize)]
pub struct ChangeRecord {
    pub facet: String,
    pub detail: String,
    /// The measure, for a measured difference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measure: Option<String>,
    /// The difference lies in `[lower, upper]`, in `unit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lower: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upper: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance: Option<f64>,
    /// `m`, `rad`, or empty for a ratio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

/// A facet that could not be compared.
#[derive(Debug, Serialize, Deserialize)]
pub struct GapRecord {
    pub facet: String,
    pub detail: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SideObject {
    pub side: String,
    pub object: ObjectId,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AmbiguousRecord {
    pub side: String,
    pub identity: String,
    pub objects: Vec<ObjectId>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SourceRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<SourceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revised: Option<SourceId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<ChangeRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved: Vec<GapRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub undetermined: Vec<ChangeRecord>,
}

/// Outcome of meshing, so a reader can tell "no finding" from "not measured".
#[derive(Debug, Serialize, Deserialize)]
pub struct GeometryRecord {
    /// Meshed with every face planar: the mesh is the shape.
    pub exact: usize,
    /// Meshed from curved faces, within the compiler's chord budget.
    pub tessellated: usize,
    /// Occupying no material, e.g. storeys, zones, openings.
    pub no_body: usize,
    /// Physical objects that could not be meshed. Geometric measurements
    /// they could affect are not evaluated.
    pub unmeasured: Vec<Unmeasured>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Unmeasured {
    pub object: ObjectId,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IntegrityRecord {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub locator: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ObjectInfo {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_id: Option<String>,
}

impl CheckOutput {
    pub fn new(
        report: Report,
        integrity: Vec<IntegrityRecord>,
        geometry: Option<GeometryRecord>,
        project: &Project,
    ) -> Self {
        let mut named: BTreeSet<&ObjectId> = BTreeSet::new();
        named.extend(
            geometry
                .iter()
                .flat_map(|g| g.unmeasured.iter().map(|u| &u.object)),
        );
        for finding in report.findings() {
            named.extend(finding.object_id());
            named.extend(&finding.related);
            named.extend(places(finding.location.as_ref()));
        }
        for outcome in report.not_evaluated() {
            named.extend(places(outcome.location.as_ref()));
        }
        named.extend(
            report
                .not_evaluated()
                .iter()
                .filter_map(NotEvaluated::object_id),
        );
        named.extend(table_objects(&report));
        let objects = named
            .into_iter()
            .filter_map(|id| {
                // A resource object the report names is labelled from the
                // report, which carries it.
                let object = report.object(project, id)?;
                Some((
                    id.to_string(),
                    ObjectInfo {
                        kind: object.kind().to_owned(),
                        global_id: object.external_id(GLOBAL_ID).map(ToOwned::to_owned),
                    },
                ))
            })
            .collect();
        Self {
            report,
            integrity,
            objects,
            geometry,
            comparison: None,
            sources: Vec::new(),
            ids: None,
            unmatched_topics: Vec::new(),
        }
    }

    /// The same result listing the BCF topics that decided no finding.
    #[must_use]
    pub fn with_unmatched_topics(mut self, topics: Vec<UnmatchedTopicRecord>) -> Self {
        self.unmatched_topics = topics;
        self
    }

    /// The same result recording the IDS document it ran.
    #[must_use]
    pub fn with_ids(mut self, ids: IdsRecord) -> Self {
        self.ids = Some(ids);
        self
    }

    /// The same result listing its sources.
    #[must_use]
    pub fn with_sources(mut self, sources: Vec<SourceInfo>) -> Self {
        self.sources = sources;
        self
    }

    /// The same result carrying what `compare` found.
    #[must_use]
    pub fn with_comparison(mut self, comparison: ComparisonRecord) -> Self {
        self.comparison = Some(comparison);
        self
    }

    /// Whether the report spans more than one source document, in which case
    /// a bare local id like `#2` is ambiguous and is qualified.
    fn several_documents(&self) -> bool {
        let scoped = self
            .report
            .findings()
            .iter()
            .map(|f| &f.scope)
            .chain(self.report.not_evaluated().iter().map(|n| &n.scope))
            .chain(
                self.report
                    .tables()
                    .iter()
                    .flat_map(|table| table.rows().iter().map(ReportRow::scope)),
            )
            .filter_map(|scope| match scope {
                Scope::Source(source) => Some(source),
                Scope::Project | Scope::Object(_) => None,
            });
        let mut documents = self.referenced().map(|id| &id.source).chain(scoped);
        let first = documents.next();
        documents.any(|other| Some(other) != first)
    }

    fn referenced(&self) -> impl Iterator<Item = &ObjectId> {
        self.report
            .findings()
            .iter()
            .flat_map(|f| f.object_id().into_iter().chain(&f.related))
            .chain(
                self.report
                    .not_evaluated()
                    .iter()
                    .filter_map(NotEvaluated::object_id),
            )
            .chain(
                self.geometry
                    .iter()
                    .flat_map(|g| g.unmeasured.iter().map(|u| &u.object)),
            )
            .chain(table_objects(&self.report))
    }

    /// `#2 IFCWALL 2O2Fr$t4X7Zf8NOew3FLOH`: local id, kind, GlobalId when known.
    fn describe(&self, id: &ObjectId, qualify: bool) -> String {
        let mut text = if qualify {
            format!("{}/{}", id.source.document, id.local_id)
        } else {
            id.local_id.clone()
        };
        if let Some(info) = self.objects.get(&id.to_string()) {
            let _ = write!(text, " {}", info.kind);
            if let Some(global_id) = &info.global_id {
                let _ = write!(text, " {global_id}");
            }
        }
        text
    }

    /// What an entry is about: the object as [`Self::describe`] names it, or
    /// `source <document>` or `project` for an entry about no single object.
    fn subject(&self, scope: &Scope, qualify: bool) -> String {
        match scope {
            Scope::Object(id) => self.describe(id, qualify),
            Scope::Source(source) => format!("source {}", source.document),
            Scope::Project => "project".to_owned(),
        }
    }

    /// Whether `location` may lie in the storey or space `query` names (by
    /// name or as [`Self::names`] reads an id): an unresolved location
    /// might, an absent one does not.
    fn located(&self, location: Option<&Location>, query: &str) -> bool {
        location.is_some_and(|location| {
            location.unresolved.is_some()
                || location.places().any(|place| {
                    place.name.as_deref() == Some(query) || self.names(&place.id, query)
                })
        })
    }

    /// Whether `query` names `id`: its full id, its local id, its local id
    /// qualified by its document (`model.ifc/#2`, as a summary over several
    /// documents prints it), or its GlobalId.
    fn names(&self, id: &ObjectId, query: &str) -> bool {
        let full = id.to_string();
        full == query
            || id.local_id == query
            || query.split_once('/').is_some_and(|(document, local)| {
                document == id.source.document && local == id.local_id
            })
            || self
                .objects
                .get(&full)
                .and_then(|info| info.global_id.as_deref())
                == Some(query)
    }
}

/// Which part of the result an entry comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Section {
    Findings,
    NotEvaluated,
    Integrity,
    /// Objects `check --geometry` could not mesh.
    Geometry,
    /// Rows of the tables of measured values rules report.
    Tables,
    /// Decisions `check --decisions` found no finding for.
    StaleDecisions,
    /// BCF topics `check --decisions-from` read that decided no finding.
    UnmatchedTopics,
}

impl Section {
    fn label(self) -> &'static str {
        match self {
            Self::Findings => "finding",
            Self::NotEvaluated => "not-evaluated",
            Self::Integrity => "integrity",
            Self::Geometry => "geometry",
            Self::Tables => "table",
            Self::StaleDecisions => "stale-decision",
            Self::UnmatchedTopics => "unmatched-topic",
        }
    }
}

/// A location as a reader reads it: `storey Level 1 (#10); space 101
/// (#20)`, and why it may be incomplete.
fn location_text(location: &Location, qualify: bool) -> String {
    let place = |kind: &str, place: &Place| {
        let id = if qualify {
            format!("{}/{}", place.id.source.document, place.id.local_id)
        } else {
            place.id.local_id.clone()
        };
        match &place.name {
            Some(name) => format!("{kind} {name} ({id})"),
            None => format!("{kind} {id}"),
        }
    };
    let mut parts: Vec<String> = location
        .storeys
        .iter()
        .map(|storey| place("storey", storey))
        .chain(location.spaces.iter().map(|space| place("space", space)))
        .collect();
    if parts.is_empty() {
        parts.push("no storey or space".to_owned());
    }
    if let Some(why) = &location.unresolved {
        parts.push(format!("unresolved: {why}"));
    }
    parts.join("; ")
}

/// The storeys and spaces of a location.
fn places(location: Option<&Location>) -> impl Iterator<Item = &ObjectId> {
    location
        .into_iter()
        .flat_map(|location| location.places().map(|place| &place.id))
}

/// The objects the rows of the report's tables are about.
fn table_objects(report: &Report) -> impl Iterator<Item = &ObjectId> {
    report
        .tables()
        .iter()
        .flat_map(|table| table.rows().iter().filter_map(|row| row.scope().object()))
}

/// A column as a summary names it: `height (m)`.
fn column_text(column: &ReportColumn) -> String {
    match column.kind.unit_symbol() {
        Some(unit) => format!("{} ({unit})", column.id),
        None => column.id.clone(),
    }
}

/// A table's columns, as the message of its summary group; a grouped
/// table's group columns first.
fn columns_text(table: &ReportTable) -> String {
    let columns: Vec<String> = table.columns().iter().map(column_text).collect();
    let columns = format!("columns: {}", columns.join(", "));
    if table.group_by().is_empty() {
        columns
    } else {
        format!("grouped by {}; {columns}", table.group_by().join(", "))
    }
}

/// A grouped row's group values as headings, `[A] [Level 1] `; nothing
/// for a row of an ungrouped table.
fn group_text(row: &ReportRow) -> String {
    row.group().iter().fold(String::new(), |mut text, value| {
        let _ = write!(text, "[{value}] ");
        text
    })
}

/// `table` as CSV (RFC 4180, `\n` line ends): a header, then one line per
/// row in table order.
///
/// The columns are `scope` (as a listing names it: `project`, `source …`
/// or an object id), the group columns, and per value column its text or,
/// for a number or quantity, `<id>_lower` and `<id>_upper` with the unit in
/// brackets: an exact value fills both, an unknown one neither.
pub fn table_csv(table: &ReportTable) -> String {
    let mut header = vec!["scope".to_owned()];
    header.extend(table.group_by().iter().cloned());
    for column in table.columns() {
        if column.kind == ReportColumnKind::Text {
            header.push(column.id.clone());
        } else {
            let unit = column
                .kind
                .unit_symbol()
                .map(|unit| format!(" [{unit}]"))
                .unwrap_or_default();
            header.push(format!("{}_lower{unit}", column.id));
            header.push(format!("{}_upper{unit}", column.id));
        }
    }
    let mut text = csv_line(&header);
    for row in table.rows() {
        let mut cells = vec![row.scope().to_string()];
        cells.extend(row.group().iter().cloned());
        for (column, value) in table.columns().iter().zip(row.values()) {
            let text = column.kind == ReportColumnKind::Text;
            match value {
                ReportValue::Text { value } => cells.push(value.clone()),
                ReportValue::Exact { value } => {
                    cells.extend([value.to_string(), value.to_string()]);
                }
                ReportValue::Interval { lower, upper } => {
                    cells.extend([lower.to_string(), upper.to_string()]);
                }
                ReportValue::Unknown if text => cells.push(String::new()),
                ReportValue::Unknown => cells.extend([String::new(), String::new()]),
            }
        }
        text.push_str(&csv_line(&cells));
    }
    text
}

/// One CSV line: fields quoted when they hold a comma, quote or line break.
fn csv_line(fields: &[String]) -> String {
    let quoted: Vec<String> = fields
        .iter()
        .map(|field| {
            if field.contains([',', '"', '\n', '\r']) {
                format!("\"{}\"", field.replace('"', "\"\""))
            } else {
                field.clone()
            }
        })
        .collect();
    quoted.join(",") + "\n"
}

/// A number as a reader reads it: at most six decimals.
fn decimal(value: f64) -> String {
    format!("{}", (value * 1e6).round() / 1e6)
}

/// One row's values, `elevation 3 m · height 3.49..3.51 m`.
fn row_text(table: &ReportTable, values: &[ReportValue]) -> String {
    table
        .columns()
        .iter()
        .zip(values)
        .map(|(column, value)| {
            let unit = column
                .kind
                .unit_symbol()
                .map(|unit| format!(" {unit}"))
                .unwrap_or_default();
            let text = match value {
                ReportValue::Unknown => "unknown".to_owned(),
                ReportValue::Exact { value } => format!("{}{unit}", decimal(*value)),
                ReportValue::Interval { lower, upper } => {
                    format!("{}..{}{unit}", decimal(*lower), decimal(*upper))
                }
                ReportValue::Text { value } => value.clone(),
            };
            format!("{} {text}", column.id)
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Entries sharing a rule (or integrity code) and a severity (or reason).
#[derive(Debug, Serialize)]
pub struct Group {
    pub section: Section,
    /// Rule id, or integrity code.
    pub key: String,
    /// Severity, or not-evaluated reason.
    pub level: String,
    pub count: usize,
    pub distinct_messages: usize,
    /// The most frequent message, elided past a fixed budget.
    pub message: String,
    /// Up to three objects the group fired on.
    pub examples: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct GeometryCounts {
    pub exact: usize,
    pub tessellated: usize,
    pub no_body: usize,
    pub unmeasured: usize,
}

/// A bounded digest of a result.
#[derive(Debug, Serialize)]
pub struct Summary {
    pub status: &'static str,
    pub findings: usize,
    pub not_evaluated: usize,
    pub integrity: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geometry: Option<GeometryCounts>,
    /// What `compare` found, when the result is a comparison.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparison: Option<ComparisonDigest>,
    /// Each rule's status, when `check --rule-status` recorded it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules: Option<RulesDigest>,
    /// Findings by decision, when `check --decisions` carried any over.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decisions: Option<DecisionCounts>,
    pub groups: Vec<Group>,
    /// Groups left out by the `top` limit, per section.
    pub omitted_groups: BTreeMap<&'static str, usize>,
    pub next: Vec<String>,
}

/// Findings by decision, and decisions whose finding is gone.
#[derive(Debug, Default, Serialize)]
pub struct DecisionCounts {
    pub accepted: usize,
    pub rejected: usize,
    pub open: usize,
    pub undecided: usize,
    /// Decided findings that changed since the decision.
    pub changed: usize,
    pub stale: usize,
}

impl DecisionCounts {
    /// `decisions: 1 accepted · … · 0 stale`, one line.
    fn line(&self) -> String {
        format!(
            "decisions: {} accepted · {} rejected · {} open · {} undecided · {} changed · {} stale\n",
            self.accepted, self.rejected, self.open, self.undecided, self.changed, self.stale
        )
    }
}

/// The decision counts of `report`; `None` when no decision was applied
/// to it.
fn decision_counts(report: &Report) -> Option<DecisionCounts> {
    let decided = report.findings().iter().any(|f| f.decision.is_some());
    if !decided && report.stale_decisions().is_empty() {
        return None;
    }
    let mut counts = DecisionCounts {
        stale: report.stale_decisions().len(),
        ..DecisionCounts::default()
    };
    for finding in report.findings() {
        match &finding.decision {
            None => counts.undecided += 1,
            Some(decision) => {
                match decision.status {
                    DecisionStatus::Accepted => counts.accepted += 1,
                    DecisionStatus::Rejected => counts.rejected += 1,
                    DecisionStatus::Open => counts.open += 1,
                }
                if decision.evidence == EvidenceCheck::Changed {
                    counts.changed += 1;
                }
            }
        }
    }
    Some(counts)
}

/// An unmatched topic's title, and what could not be read of it.
fn unmatched_message(topic: &UnmatchedTopicRecord) -> String {
    let title = topic.title.as_deref().unwrap_or("(untitled topic)");
    match &topic.detail {
        Some(detail) => format!("{title}: {detail}"),
        None => title.to_owned(),
    }
}

/// A stale decision's rule and message, from its basis; `-` and the
/// finding's identity without one.
fn stale_subject(decision: &Decision) -> (String, String) {
    match &decision.basis {
        Some(basis) => (basis.rule_id.to_string(), basis.message.clone()),
        None => ("-".to_owned(), format!("finding {}", decision.finding)),
    }
}

/// `accepted by A. Reviewer on 2026-09-27T08:00:00Z: comment`: the latest
/// comment, prefixed by its author when someone else wrote it, and how many
/// came before it.
fn decision_text(
    status: DecisionStatus,
    author: &str,
    date: &axioval::ir::DateTime,
    comments: &[DecisionComment],
) -> String {
    let mut text = format!("{} by {author} on {date}", status.as_str());
    if let Some(latest) = comments.last() {
        let said = latest.text.trim();
        if latest.author == author {
            let _ = write!(text, ": {said}");
        } else {
            let _ = write!(text, ": {}: {said}", latest.author);
        }
        if comments.len() > 1 {
            let _ = write!(text, " (+{} earlier comment(s))", comments.len() - 1);
        }
    }
    text
}

/// `; assigned to A; due D; priority P; labels a, b`, each part only when
/// set.
fn review_text(
    assigned_to: Option<&str>,
    due_date: Option<&axioval::ir::DateTime>,
    priority: Option<&str>,
    labels: &[String],
) -> String {
    let mut text = String::new();
    if let Some(assignee) = assigned_to {
        let _ = write!(text, "; assigned to {assignee}");
    }
    if let Some(due) = due_date {
        let _ = write!(text, "; due {due}");
    }
    if let Some(priority) = priority {
        let _ = write!(text, "; priority {priority}");
    }
    if !labels.is_empty() {
        let _ = write!(text, "; labels {}", labels.join(", "));
    }
    text
}

fn carried_text(decision: &FindingDecision) -> String {
    let mut text = decision_text(
        decision.status,
        &decision.author,
        &decision.date,
        &decision.comments,
    );
    text.push_str(&review_text(
        decision.assigned_to.as_deref(),
        decision.due_date.as_ref(),
        decision.priority.as_deref(),
        &decision.labels,
    ));
    match decision.evidence {
        EvidenceCheck::Unchanged => {}
        EvidenceCheck::Unknown => text.push_str(" (changes unknown: no basis recorded)"),
        EvidenceCheck::Changed => {
            let changes: Vec<String> = decision
                .changes
                .iter()
                .map(|c| format!("{} {} -> {}", c.facet.as_str(), c.decided, c.now))
                .collect();
            let _ = write!(text, " (changed since: {})", changes.join("; "));
        }
    }
    text
}

/// Rules by status: how many of each, and the rules themselves, those that
/// did not pass first, at most `top`.
#[derive(Debug, Serialize)]
pub struct RulesDigest {
    pub counts: BTreeMap<&'static str, usize>,
    pub rules: Vec<RuleLine>,
    /// Rules left out by the `top` limit.
    pub omitted: usize,
}

/// One rule's counts.
#[derive(Debug, Serialize)]
pub struct RuleLine {
    pub rule: String,
    pub status: &'static str,
    pub checked: usize,
    pub failed: usize,
    pub not_evaluated: usize,
}

fn rule_status(status: RuleStatus) -> &'static str {
    match status {
        RuleStatus::Failed => "failed",
        RuleStatus::NotEvaluated => "not evaluated",
        RuleStatus::NothingSelected => "nothing selected",
        RuleStatus::Passed => "passed",
        RuleStatus::Skipped => "skipped",
    }
}

/// The rules digest of `report`; `None` when it records no rule status.
fn rules_digest(report: &Report, top: usize) -> Option<RulesDigest> {
    if report.rules().is_empty() {
        return None;
    }
    let rank = |status: RuleStatus| match status {
        RuleStatus::Failed => 0,
        RuleStatus::NotEvaluated => 1,
        RuleStatus::NothingSelected => 2,
        RuleStatus::Passed => 3,
        RuleStatus::Skipped => 4,
    };
    let mut ordered: Vec<_> = report.rules().iter().collect();
    ordered.sort_by(|a, b| {
        rank(a.status)
            .cmp(&rank(b.status))
            .then_with(|| a.rule_id.cmp(&b.rule_id))
    });
    let mut counts = BTreeMap::new();
    for summary in &ordered {
        *counts.entry(rule_status(summary.status)).or_default() += 1;
    }
    let rules: Vec<RuleLine> = ordered
        .iter()
        .take(top)
        .map(|summary| RuleLine {
            rule: summary.rule_id.to_string(),
            status: rule_status(summary.status),
            checked: summary.checked,
            failed: summary.failed,
            not_evaluated: summary.not_evaluated,
        })
        .collect();
    Some(RulesDigest {
        counts,
        omitted: ordered.len() - rules.len(),
        rules,
    })
}

/// The head of a comparison's summary.
#[derive(Debug, Serialize)]
pub struct ComparisonDigest {
    pub base: String,
    pub revised: String,
    pub facets: Vec<String>,
    pub counts: ComparisonCounts,
}

pub fn status(report: &Report) -> &'static str {
    if !report.findings().is_empty() {
        "findings"
    } else if !report.not_evaluated().is_empty() {
        "incomplete"
    } else {
        "passed"
    }
}

fn severity(severity: &Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

fn reason(reason: &NotEvaluatedReason) -> &'static str {
    match reason {
        NotEvaluatedReason::MissingService => "missing-service",
        NotEvaluatedReason::BackendUnavailable => "backend-unavailable",
        NotEvaluatedReason::IncompleteEvidence => "incomplete-evidence",
        NotEvaluatedReason::InvalidEvidence => "invalid-evidence",
        NotEvaluatedReason::InvalidDeclaration => "invalid-declaration",
        NotEvaluatedReason::UnboundConcept => "unbound-concept",
        NotEvaluatedReason::NotRecorded => "not-recorded",
        NotEvaluatedReason::ResourceLimit => "resource-limit",
    }
}

fn elide(message: &str, budget: usize) -> String {
    let message = message.trim();
    match message.char_indices().nth(budget) {
        Some((cut, _)) => format!("{}…", &message[..cut]),
        None => message.to_owned(),
    }
}

/// `message` with every `#<digits>` instance reference replaced by `#…`.
///
/// Messages that differ only in which instance they name say the same thing;
/// counting them as distinct would report 95 kinds of one problem.
fn shape(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    let mut chars = message.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '#' && chars.peek().is_some_and(char::is_ascii_digit) {
            while chars.peek().is_some_and(char::is_ascii_digit) {
                chars.next();
            }
            out.push('…');
        }
    }
    out
}

/// Accumulates one group while scanning.
#[derive(Default)]
struct Tally {
    count: usize,
    /// Message shape -> (occurrences, first message of that shape).
    messages: BTreeMap<String, (usize, String)>,
    examples: Vec<String>,
}

impl Tally {
    fn add(&mut self, message: &str, example: Option<String>) {
        self.count += 1;
        let message = message.trim();
        self.messages
            .entry(shape(message))
            .or_insert_with(|| (0, message.to_owned()))
            .0 += 1;
        if let Some(example) = example
            && self.examples.len() < EXAMPLES
        {
            self.examples.push(example);
        }
    }

    fn into_group(self, section: Section, key: String, level: String) -> Group {
        // Most frequent first; ties by text, so the choice is deterministic.
        let message = self
            .messages
            .iter()
            .max_by(|a, b| a.1.0.cmp(&b.1.0).then_with(|| b.0.cmp(a.0)))
            .map(|(_, (_, text))| elide(text, MESSAGE_BUDGET))
            .unwrap_or_default();
        Group {
            section,
            key,
            level,
            count: self.count,
            distinct_messages: self.messages.len(),
            message,
            examples: self.examples,
        }
    }
}

/// Digests `output`, keeping at most `top` groups per section.
///
/// `saved` is where the full result is, for the drill-down hints; `None` when
/// it was not written to a file and so cannot be queried.
/// Tallies every entry of `output` by section, key and level.
fn tally(output: &CheckOutput) -> BTreeMap<(Section, String, String), Tally> {
    let qualify = output.several_documents();
    let mut tallies: BTreeMap<(Section, String, String), Tally> = BTreeMap::new();
    for finding in output.report.findings() {
        tallies
            .entry((
                Section::Findings,
                finding.rule_id.to_string(),
                severity(&finding.severity).to_owned(),
            ))
            .or_default()
            .add(
                &finding.message,
                // A source or project finding names its scope in its
                // message; an example is always an object to drill into.
                finding.object_id().map(|id| output.describe(id, qualify)),
            );
    }
    for outcome in output.report.not_evaluated() {
        tallies
            .entry((
                Section::NotEvaluated,
                outcome.rule_id.to_string(),
                reason(&outcome.reason).to_owned(),
            ))
            .or_default()
            .add(
                &outcome.message,
                outcome.object_id().map(|id| output.describe(id, qualify)),
            );
    }
    for record in &output.integrity {
        tallies
            .entry((
                Section::Integrity,
                record.code.clone(),
                record.severity.clone(),
            ))
            .or_default()
            .add(&record.message, None);
    }
    for unmeasured in output.geometry.iter().flat_map(|g| &g.unmeasured) {
        // Grouped by the kind of failure, the text before any detail.
        let class = unmeasured
            .reason
            .split(':')
            .next()
            .unwrap_or(&unmeasured.reason)
            .trim()
            .to_owned();
        tallies
            .entry((Section::Geometry, class, "unmeasured".to_owned()))
            .or_default()
            .add(
                &unmeasured.reason,
                Some(output.describe(&unmeasured.object, qualify)),
            );
    }
    // Stale decisions by the rule and status they were recorded with.
    for decision in output.report.stale_decisions() {
        let (rule, message) = stale_subject(decision);
        tallies
            .entry((
                Section::StaleDecisions,
                rule,
                decision.status.as_str().to_owned(),
            ))
            .or_default()
            .add(&message, None);
    }
    // Unmatched topics by why they decided nothing and their status.
    for topic in &output.unmatched_topics {
        tallies
            .entry((
                Section::UnmatchedTopics,
                topic.reason.clone(),
                topic.status.clone().unwrap_or_else(|| "-".to_owned()),
            ))
            .or_default()
            .add(&unmatched_message(topic), None);
    }
    // One group per table: its rows are the count, its columns the message.
    for table in output.report.tables() {
        let columns = columns_text(table);
        let tally = tallies
            .entry((
                Section::Tables,
                table.rule_id().to_string(),
                table.name().to_owned(),
            ))
            .or_default();
        for row in table.rows() {
            tally.add(
                &columns,
                row.scope().object().map(|id| output.describe(id, qualify)),
            );
        }
    }

    tallies
}

pub fn summarize(output: &CheckOutput, top: usize, saved: Option<&str>) -> Summary {
    let tallies = tally(output);
    let mut groups: Vec<Group> = tallies
        .into_iter()
        .map(|((section, key, level), tally)| tally.into_group(section, key, level))
        .collect();
    // Within a section: the most severe level first, then the largest group.
    let rank = |level: &str| match level {
        "error" => 0,
        "warning" => 1,
        _ => 2,
    };
    groups.sort_by(|a, b| {
        a.section
            .cmp(&b.section)
            .then(rank(&a.level).cmp(&rank(&b.level)))
            .then(b.count.cmp(&a.count))
            .then_with(|| a.key.cmp(&b.key))
    });
    let mut kept = Vec::new();
    let mut omitted_groups = BTreeMap::new();
    let mut shown: BTreeMap<Section, usize> = BTreeMap::new();
    for group in groups {
        let seen = shown.entry(group.section).or_default();
        if *seen < top {
            *seen += 1;
            kept.push(group);
        } else {
            *omitted_groups.entry(group.section.label()).or_default() += 1;
        }
    }

    let mut next = next_steps(&kept, &omitted_groups, top, saved);
    let missing_service = output
        .report
        .not_evaluated()
        .iter()
        .any(|n| n.reason == NotEvaluatedReason::MissingService);
    if output.geometry.is_none() && missing_service {
        let command = if output.comparison.is_some() {
            "compare"
        } else {
            "check"
        };
        next.push(format!(
            "some rules lack an evidence service; if they are geometric, rerun `axioval {command}` with --geometry"
        ));
    }
    Summary {
        status: status(&output.report),
        findings: output.report.findings().len(),
        not_evaluated: output.report.not_evaluated().len(),
        integrity: output.integrity.len(),
        geometry: output.geometry.as_ref().map(|g| GeometryCounts {
            exact: g.exact,
            tessellated: g.tessellated,
            no_body: g.no_body,
            unmeasured: g.unmeasured.len(),
        }),
        comparison: output.comparison.as_ref().map(|c| ComparisonDigest {
            base: c.base.clone(),
            revised: c.revised.clone(),
            facets: c.facets.clone(),
            counts: c.counts,
        }),
        rules: rules_digest(&output.report, top),
        decisions: decision_counts(&output.report),
        groups: kept,
        omitted_groups,
        next,
    }
}

fn next_steps(
    groups: &[Group],
    omitted: &BTreeMap<&'static str, usize>,
    top: usize,
    saved: Option<&str>,
) -> Vec<String> {
    let Some(path) = saved else {
        return if groups.is_empty() {
            vec![]
        } else {
            vec!["rerun with --report <file> to list entries with `axioval report`".into()]
        };
    };
    let quoted = shell_quote(path);
    let mut next = Vec::new();
    if let Some(group) = groups.first() {
        next.push(match group.section {
            Section::Integrity => {
                format!("axioval report {quoted} --code {}", shell_quote(&group.key))
            }
            Section::Geometry => format!("axioval report {quoted} --section geometry"),
            Section::StaleDecisions => {
                format!("axioval report {quoted} --section stale-decisions")
            }
            Section::UnmatchedTopics => {
                format!("axioval report {quoted} --section unmatched-topics")
            }
            Section::Tables => format!(
                "axioval report {quoted} --section tables --rule {}",
                shell_quote(&group.key)
            ),
            _ => format!("axioval report {quoted} --rule {}", shell_quote(&group.key)),
        });
    }
    // Unmeasured bodies qualify every geometric answer, so they get their own
    // step even when a larger group comes first.
    if groups
        .first()
        .is_some_and(|g| g.section != Section::Geometry)
        && groups.iter().any(|g| g.section == Section::Geometry)
    {
        next.push(format!("axioval report {quoted} --section geometry"));
    }
    // A stale decision is a reviewed finding that is gone: worth a look.
    if groups
        .first()
        .is_some_and(|g| g.section != Section::StaleDecisions)
        && groups.iter().any(|g| g.section == Section::StaleDecisions)
    {
        next.push(format!("axioval report {quoted} --section stale-decisions"));
    }
    // A topic that decided nothing may be a review that went nowhere.
    if groups
        .first()
        .is_some_and(|g| g.section != Section::UnmatchedTopics)
        && groups.iter().any(|g| g.section == Section::UnmatchedTopics)
    {
        next.push(format!(
            "axioval report {quoted} --section unmatched-topics"
        ));
    }
    // Measured values are what a reader asks for next once issues are known.
    if groups.first().is_some_and(|g| g.section != Section::Tables)
        && groups.iter().any(|g| g.section == Section::Tables)
    {
        next.push(format!("axioval report {quoted} --section tables"));
    }
    if let Some(example) = groups.iter().flat_map(|g| &g.examples).next() {
        let id = example.split(' ').next().unwrap_or(example);
        next.push(format!(
            "axioval report {quoted} --object {} --evidence",
            shell_quote(id)
        ));
    }
    if !omitted.is_empty() {
        next.push(format!("axioval report {quoted} --top {}", top * 5));
    }
    next
}

/// `text` as one POSIX shell word. `#` and `$` are quoted: an unquoted `#`
/// starts a comment and `$` expands, and both occur in ids.
pub fn shell_quote(text: &str) -> String {
    if text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./:".contains(c))
    {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

pub fn render_summary(summary: &Summary) -> String {
    let mut out = String::new();
    // A comparison says first what was compared with what.
    if let Some(comparison) = &summary.comparison {
        let counts = comparison.counts;
        let _ = writeln!(
            out,
            "compared: {} -> {} · {}",
            comparison.base,
            comparison.revised,
            comparison.facets.join(", ")
        );
        let _ = writeln!(
            out,
            "objects: {} added · {} removed · {} changed · {} unchanged · {} incomplete · {} unidentified · {} ambiguous",
            counts.added,
            counts.removed,
            counts.changed,
            counts.unchanged,
            counts.incomplete,
            counts.unidentified,
            counts.ambiguous
        );
    }
    let _ = writeln!(
        out,
        "status: {} · {} finding(s) · {} not evaluated · {} integrity issue(s)",
        summary.status, summary.findings, summary.not_evaluated, summary.integrity
    );
    if let Some(geometry) = &summary.geometry {
        let _ = writeln!(
            out,
            "geometry: {} exact · {} tessellated · {} without body · {} unmeasured",
            geometry.exact, geometry.tessellated, geometry.no_body, geometry.unmeasured
        );
    }
    if let Some(rules) = &summary.rules {
        let counts: Vec<String> = ["failed", "not evaluated", "nothing selected", "passed"]
            .iter()
            .filter_map(|status| {
                rules
                    .counts
                    .get(status)
                    .map(|count| format!("{count} {status}"))
            })
            .collect();
        let _ = writeln!(out, "\nrules: {}", counts.join(" · "));
        for rule in &rules.rules {
            let _ = writeln!(
                out,
                "  {:<16}  {:>6} checked · {} failed · {} not evaluated  {}",
                rule.status, rule.checked, rule.failed, rule.not_evaluated, rule.rule
            );
        }
        if rules.omitted > 0 {
            let _ = writeln!(out, "  … {} more rule(s)", rules.omitted);
        }
    }
    if let Some(decisions) = &summary.decisions {
        out.push_str(&decisions.line());
    }
    let mut section = None;
    for group in &summary.groups {
        if section != Some(group.section) {
            section = Some(group.section);
            let _ = writeln!(out, "\n{}:", group.section.label());
        }
        let _ = write!(
            out,
            "  {:>6}  {:<7}  {}",
            group.count, group.level, group.key
        );
        if group.distinct_messages > 1 {
            let _ = write!(out, "  ({} distinct messages)", group.distinct_messages);
        }
        let _ = writeln!(out, "\n          {}", group.message);
        if !group.examples.is_empty() {
            let more = group.count.saturating_sub(group.examples.len());
            let _ = write!(out, "          e.g. {}", group.examples.join("; "));
            if more > 0 {
                let _ = write!(out, "; +{more} more");
            }
            out.push('\n');
        }
    }
    for (section, count) in &summary.omitted_groups {
        let _ = writeln!(out, "  … {count} more {section} group(s)");
    }
    if !summary.next.is_empty() {
        out.push_str("\nnext:\n");
        for step in &summary.next {
            let _ = writeln!(out, "  {step}");
        }
    }
    out
}

/// What a listing selects. Every present filter must match.
pub struct Filter {
    pub section: Option<Section>,
    pub rule: Option<String>,
    pub code: Option<String>,
    pub object: Option<String>,
    /// A storey or space; only located findings and outcomes match.
    pub location: Option<String>,
    /// A decision; only findings match.
    pub decision: Option<DecisionFilter>,
}

/// The `report --decision` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum DecisionFilter {
    Accepted,
    Rejected,
    Open,
    /// No decision.
    Undecided,
    /// Decided, and changed since the decision.
    Changed,
}

impl DecisionFilter {
    fn matches(self, decision: Option<&FindingDecision>) -> bool {
        match (self, decision) {
            (Self::Undecided, None) => true,
            (_, None) | (Self::Undecided, Some(_)) => false,
            (Self::Accepted, Some(d)) => d.status == DecisionStatus::Accepted,
            (Self::Rejected, Some(d)) => d.status == DecisionStatus::Rejected,
            (Self::Open, Some(d)) => d.status == DecisionStatus::Open,
            (Self::Changed, Some(d)) => d.evidence == EvidenceCheck::Changed,
        }
    }
}

/// One listed entry.
#[derive(Debug, Serialize)]
pub struct Entry {
    pub section: Section,
    /// Rule id, or integrity code.
    pub key: String,
    /// Severity, or not-evaluated reason.
    pub level: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    /// `source <document>` or `project` for an entry about no single object.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
    /// The storeys and spaces a located entry lies in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    pub message: String,
    /// The finding's identity, to record a decision against.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The decision about the finding, as text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Listing {
    pub total: usize,
    pub offset: usize,
    pub entries: Vec<Entry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

#[allow(clippy::too_many_lines)]
/// Entries matching `filter`, paged. Evidence locators are long and rarely
/// needed, so they are included only on request.
pub fn list(
    output: &CheckOutput,
    filter: &Filter,
    offset: usize,
    limit: usize,
    evidence: bool,
    command: &str,
) -> Listing {
    let qualify = output.several_documents();
    let wants = |section: Section| filter.section.is_none_or(|wanted| wanted == section);
    let rule_ok = |rule: &str| filter.rule.as_deref().is_none_or(|wanted| wanted == rule);
    let place_ok = |location: Option<&Location>| {
        filter
            .location
            .as_deref()
            .is_none_or(|query| output.located(location, query))
    };
    // Only findings and not-evaluated outcomes are located, and only
    // findings decided.
    let unlocated = filter.location.is_none() && filter.decision.is_none();
    let decision_ok = |decision: Option<&FindingDecision>| {
        filter
            .decision
            .is_none_or(|wanted| wanted.matches(decision))
    };
    let mut matched: Vec<Entry> = Vec::new();

    // `--code` names integrity issues only; rules never match it.
    if filter.code.is_none() {
        if wants(Section::Findings) {
            let findings = output.report.findings().iter().filter(|finding| {
                rule_ok(&finding.rule_id.to_string())
                    && place_ok(finding.location.as_ref())
                    && decision_ok(finding.decision.as_ref())
                    && filter.object.as_deref().is_none_or(|query| {
                        names_source(&finding.scope, query)
                            || finding
                                .object_id()
                                .into_iter()
                                .chain(&finding.related)
                                .any(|id| output.names(id, query))
                    })
            });
            matched.extend(findings.map(|f| finding_entry(output, f, qualify, evidence)));
        }
        if wants(Section::NotEvaluated) && filter.decision.is_none() {
            let outcomes = output.report.not_evaluated().iter().filter(|outcome| {
                rule_ok(&outcome.rule_id.to_string())
                    && place_ok(outcome.location.as_ref())
                    && filter.object.as_deref().is_none_or(|query| {
                        names_source(&outcome.scope, query)
                            || outcome
                                .object_id()
                                .is_some_and(|id| output.names(id, query))
                    })
            });
            matched.extend(outcomes.map(|o| not_evaluated_entry(output, o, qualify)));
        }
    }
    if unlocated && filter.rule.is_none() && wants(Section::Integrity) {
        let records = output.integrity.iter().filter(|record| {
            filter
                .code
                .as_deref()
                .is_none_or(|code| code == record.code)
                && filter
                    .object
                    .as_deref()
                    .is_none_or(|query| mentions(&record.message, query))
        });
        matched.extend(records.map(|record| Entry {
            id: None,
            decision: None,
            section: Section::Integrity,
            key: record.code.clone(),
            level: record.severity.clone(),
            object: None,
            scope: None,
            related: vec![],
            location: None,
            message: record.message.trim().to_owned(),
            evidence: if evidence {
                vec![record.locator.clone()]
            } else {
                vec![]
            },
        }));
    }
    if unlocated && filter.rule.is_none() && filter.code.is_none() && wants(Section::Geometry) {
        let unmeasured = output
            .geometry
            .iter()
            .flat_map(|g| &g.unmeasured)
            .filter(|u| {
                filter
                    .object
                    .as_deref()
                    .is_none_or(|query| output.names(&u.object, query))
            });
        matched.extend(unmeasured.map(|u| Entry {
            id: None,
            decision: None,
            section: Section::Geometry,
            key: "unmeasured".to_owned(),
            level: "unmeasured".to_owned(),
            object: Some(output.describe(&u.object, qualify)),
            scope: None,
            related: vec![],
            location: None,
            message: u.reason.clone(),
            evidence: vec![],
        }));
    }

    if unlocated && filter.code.is_none() && wants(Section::Tables) {
        matched.extend(table_entries(output, filter, qualify));
    }

    // A stale decision names no object that is still reported.
    if unlocated
        && filter.code.is_none()
        && filter.object.is_none()
        && wants(Section::StaleDecisions)
    {
        for decision in output.report.stale_decisions() {
            let (rule, message) = stale_subject(decision);
            if !rule_ok(&rule) {
                continue;
            }
            matched.push(Entry {
                section: Section::StaleDecisions,
                key: rule,
                level: decision.status.as_str().to_owned(),
                object: None,
                scope: None,
                related: vec![],
                location: None,
                message,
                id: Some(decision.finding.to_string()),
                decision: Some(
                    decision_text(
                        decision.status,
                        &decision.author,
                        &decision.date,
                        &decision.comments,
                    ) + &review_text(
                        decision.assigned_to.as_deref(),
                        decision.due_date.as_ref(),
                        decision.priority.as_deref(),
                        &decision.labels,
                    ),
                ),
                evidence: vec![],
            });
        }
    }

    // An unmatched topic names no rule and no object of the report.
    if unlocated
        && filter.code.is_none()
        && filter.object.is_none()
        && filter.rule.is_none()
        && wants(Section::UnmatchedTopics)
    {
        for topic in &output.unmatched_topics {
            matched.push(Entry {
                section: Section::UnmatchedTopics,
                key: topic.reason.clone(),
                level: topic.status.clone().unwrap_or_else(|| "-".to_owned()),
                object: None,
                scope: None,
                related: vec![],
                location: None,
                message: unmatched_message(topic),
                id: topic.guid.clone(),
                decision: None,
                evidence: vec![],
            });
        }
    }

    let total = matched.len();
    let entries: Vec<Entry> = matched.into_iter().skip(offset).take(limit).collect();
    let end = offset + entries.len();
    let next = (end < total).then(|| format!("{command} --offset {end}"));
    Listing {
        total,
        offset,
        entries,
        next,
    }
}

/// The rows of the report's tables that `filter`'s rule and object select,
/// in table order.
fn table_entries(output: &CheckOutput, filter: &Filter, qualify: bool) -> Vec<Entry> {
    let mut entries = Vec::new();
    for table in output.report.tables() {
        if filter
            .rule
            .as_deref()
            .is_some_and(|wanted| wanted != table.rule_id().to_string())
        {
            continue;
        }
        let rows = table.rows().iter().filter(|row| {
            filter.object.as_deref().is_none_or(|query| {
                names_source(row.scope(), query)
                    || row
                        .scope()
                        .object()
                        .is_some_and(|id| output.names(id, query))
            })
        });
        entries.extend(rows.map(|row| {
            let (object, scope) = subject_fields(output, row.scope(), qualify);
            Entry {
                id: None,
                decision: None,
                section: Section::Tables,
                key: table.rule_id().to_string(),
                level: table.name().to_owned(),
                object,
                scope,
                related: vec![],
                location: None,
                message: format!("{}{}", group_text(row), row_text(table, row.values())),
                evidence: vec![],
            }
        }));
    }
    entries
}

/// Whether `query` names the source a source-scoped entry is about, by its
/// full id or its document.
fn names_source(scope: &Scope, query: &str) -> bool {
    match scope {
        Scope::Source(SourceId { system, document }) => {
            query == document || query == format!("{system}:{document}")
        }
        Scope::Project | Scope::Object(_) => false,
    }
}

/// The `object` and `scope` fields of an entry about `scope`.
fn subject_fields(
    output: &CheckOutput,
    scope: &Scope,
    qualify: bool,
) -> (Option<String>, Option<String>) {
    let text = output.subject(scope, qualify);
    match scope {
        Scope::Object(_) => (Some(text), None),
        Scope::Source(_) | Scope::Project => (None, Some(text)),
    }
}

/// Whether `message` names `local` as a whole token: `#12` is not in `#123`.
fn mentions(message: &str, local: &str) -> bool {
    message.match_indices(local).any(|(at, _)| {
        let before = message[..at].chars().next_back();
        let after = message[at + local.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

fn finding_entry(output: &CheckOutput, finding: &Finding, qualify: bool, evidence: bool) -> Entry {
    let (object, scope) = subject_fields(output, &finding.scope, qualify);
    Entry {
        section: Section::Findings,
        key: finding.rule_id.to_string(),
        level: severity(&finding.severity).to_owned(),
        object,
        scope,
        related: finding
            .related
            .iter()
            .map(|id| output.describe(id, qualify))
            .collect(),
        location: finding
            .location
            .as_ref()
            .map(|location| location_text(location, qualify)),
        message: finding.message.trim().to_owned(),
        id: finding.id.map(|id| id.to_string()),
        decision: finding.decision.as_ref().map(carried_text),
        evidence: if evidence {
            finding
                .evidence
                .iter()
                .map(|e| {
                    let exactness = if e.exact { "exact" } else { "inexact" };
                    format!("{} ({exactness})", e.locator)
                })
                .collect()
        } else {
            vec![]
        },
    }
}

fn not_evaluated_entry(output: &CheckOutput, outcome: &NotEvaluated, qualify: bool) -> Entry {
    let (object, scope) = subject_fields(output, &outcome.scope, qualify);
    Entry {
        id: None,
        decision: None,
        section: Section::NotEvaluated,
        key: outcome.rule_id.to_string(),
        level: reason(&outcome.reason).to_owned(),
        object,
        scope,
        related: vec![],
        location: outcome
            .location
            .as_ref()
            .map(|location| location_text(location, qualify)),
        message: outcome.message.trim().to_owned(),
        evidence: vec![],
    }
}

pub fn render_listing(listing: &Listing) -> String {
    let mut out = String::new();
    for entry in &listing.entries {
        let _ = write!(
            out,
            "[{}] {} {}",
            entry.section.label(),
            entry.level,
            entry.key
        );
        if let Some(object) = &entry.object {
            let _ = write!(out, "  {object}");
        }
        if let Some(scope) = &entry.scope {
            let _ = write!(out, "  ({scope})");
        }
        let _ = writeln!(out, "\n    {}", entry.message);
        if let Some(id) = &entry.id {
            let _ = writeln!(out, "    id: {id}");
        }
        if let Some(decision) = &entry.decision {
            let _ = writeln!(out, "    decision: {decision}");
        }
        if !entry.related.is_empty() {
            let _ = writeln!(out, "    related: {}", entry.related.join("; "));
        }
        if let Some(location) = &entry.location {
            let _ = writeln!(out, "    location: {location}");
        }
        for evidence in &entry.evidence {
            let _ = writeln!(out, "    evidence: {evidence}");
        }
    }
    if listing.total == 0 {
        out.push_str("no matching entries\n");
    } else {
        let _ = writeln!(
            out,
            "showing {}–{} of {}",
            listing.offset + 1,
            listing.offset + listing.entries.len(),
            listing.total
        );
    }
    if let Some(next) = &listing.next {
        let _ = writeln!(out, "next: {next}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{elide, mentions, shape};

    #[test]
    fn messages_differing_only_in_instances_share_a_shape() {
        assert_eq!(
            shape("#157259 has no end; see #12"),
            shape("#204035 has no end; see #9")
        );
        assert_ne!(shape("#1 has no end"), shape("#1 has no start"));
        assert_eq!(shape("C# and #x stay"), "C# and #x stay");
    }

    #[test]
    fn token_mentions_do_not_match_prefixes() {
        assert!(mentions("#12 has GlobalId 'x'", "#12"));
        assert!(mentions("claimed by #3, #12", "#12"));
        assert!(!mentions("#123 has GlobalId 'x'", "#12"));
        assert!(!mentions("#1242", "#12"));
    }

    #[test]
    fn elision_counts_characters_not_bytes() {
        assert_eq!(elide("äöü", 2), "äö…");
        assert_eq!(elide("  short  ", 10), "short");
    }
}
