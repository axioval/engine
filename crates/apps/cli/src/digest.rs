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
    Finding, NotEvaluated, NotEvaluatedReason, ObjectId, Project, Report, RuleFinding, Severity,
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
            named.insert(&finding.object_id);
            named.extend(&finding.related);
        }
        for finding in report.rule_findings() {
            named.extend(&finding.related);
        }
        named.extend(
            report
                .not_evaluated()
                .iter()
                .filter_map(|n| n.object_id.as_ref()),
        );
        let objects = named
            .into_iter()
            .filter_map(|id| {
                let object = project.object(id)?;
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
        }
    }

    /// Whether the report spans more than one source document, in which case
    /// a bare local id like `#2` is ambiguous and is qualified.
    fn several_documents(&self) -> bool {
        let mut documents = self.referenced().map(|id| &id.source);
        let first = documents.next();
        documents.any(|other| Some(other) != first)
    }

    fn referenced(&self) -> impl Iterator<Item = &ObjectId> {
        self.report
            .findings()
            .iter()
            .flat_map(|f| std::iter::once(&f.object_id).chain(&f.related))
            .chain(
                self.report
                    .rule_findings()
                    .iter()
                    .flat_map(|f| f.related.iter()),
            )
            .chain(
                self.report
                    .not_evaluated()
                    .iter()
                    .filter_map(|n| n.object_id.as_ref()),
            )
            .chain(
                self.geometry
                    .iter()
                    .flat_map(|g| g.unmeasured.iter().map(|u| &u.object)),
            )
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

    /// Whether `query` names `id`: its full id, its local id, or its GlobalId.
    fn names(&self, id: &ObjectId, query: &str) -> bool {
        let full = id.to_string();
        full == query
            || id.local_id == query
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
}

impl Section {
    fn label(self) -> &'static str {
        match self {
            Self::Findings => "finding",
            Self::NotEvaluated => "not-evaluated",
            Self::Integrity => "integrity",
            Self::Geometry => "geometry",
        }
    }
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
    pub groups: Vec<Group>,
    /// Groups left out by the `top` limit, per section.
    pub omitted_groups: BTreeMap<&'static str, usize>,
    pub next: Vec<String>,
}

pub fn status(report: &Report) -> &'static str {
    if report.has_findings() {
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
                Some(output.describe(&finding.object_id, qualify)),
            );
    }
    // A finding about a whole population fires on the rule, not an object;
    // its participants, when it has any, are the examples.
    for finding in output.report.rule_findings() {
        let tally = tallies
            .entry((
                Section::Findings,
                finding.rule_id.to_string(),
                severity(&finding.severity).to_owned(),
            ))
            .or_default();
        let mut related = finding.related.iter();
        tally.add(
            &finding.message,
            related.next().map(|id| output.describe(id, qualify)),
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
                outcome
                    .object_id
                    .as_ref()
                    .map(|id| output.describe(id, qualify)),
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
        next.push(
            "some rules lack an evidence service; if they are geometric, rerun `axioval check` with --geometry"
                .into(),
        );
    }
    Summary {
        status: status(&output.report),
        findings: output.report.findings().len() + output.report.rule_findings().len(),
        not_evaluated: output.report.not_evaluated().len(),
        integrity: output.integrity.len(),
        geometry: output.geometry.as_ref().map(|g| GeometryCounts {
            exact: g.exact,
            tessellated: g.tessellated,
            no_body: g.no_body,
            unmeasured: g.unmeasured.len(),
        }),
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
    let mut out = format!(
        "status: {} · {} finding(s) · {} not evaluated · {} integrity issue(s)\n",
        summary.status, summary.findings, summary.not_evaluated, summary.integrity
    );
    if let Some(geometry) = &summary.geometry {
        let _ = writeln!(
            out,
            "geometry: {} exact · {} tessellated · {} without body · {} unmeasured",
            geometry.exact, geometry.tessellated, geometry.no_body, geometry.unmeasured
        );
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
    pub message: String,
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
    let mut matched: Vec<Entry> = Vec::new();

    // `--code` names integrity issues only; rules never match it.
    if filter.code.is_none() {
        if wants(Section::Findings) {
            let findings = output.report.findings().iter().filter(|finding| {
                rule_ok(&finding.rule_id.to_string())
                    && filter.object.as_deref().is_none_or(|query| {
                        std::iter::once(&finding.object_id)
                            .chain(&finding.related)
                            .any(|id| output.names(id, query))
                    })
            });
            matched.extend(findings.map(|f| finding_entry(output, f, qualify, evidence)));
            let populations = output.report.rule_findings().iter().filter(|finding| {
                rule_ok(&finding.rule_id.to_string())
                    && filter.object.as_deref().is_none_or(|query| {
                        finding.related.iter().any(|id| output.names(id, query))
                    })
            });
            matched.extend(populations.map(|f| rule_finding_entry(output, f, qualify, evidence)));
        }
        if wants(Section::NotEvaluated) {
            let outcomes = output.report.not_evaluated().iter().filter(|outcome| {
                rule_ok(&outcome.rule_id.to_string())
                    && filter.object.as_deref().is_none_or(|query| {
                        outcome
                            .object_id
                            .as_ref()
                            .is_some_and(|id| output.names(id, query))
                    })
            });
            matched.extend(outcomes.map(|o| not_evaluated_entry(output, o, qualify)));
        }
    }
    if filter.rule.is_none() && wants(Section::Integrity) {
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
            section: Section::Integrity,
            key: record.code.clone(),
            level: record.severity.clone(),
            object: None,
            related: vec![],
            message: record.message.trim().to_owned(),
            evidence: if evidence {
                vec![record.locator.clone()]
            } else {
                vec![]
            },
        }));
    }
    if filter.rule.is_none() && filter.code.is_none() && wants(Section::Geometry) {
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
            section: Section::Geometry,
            key: "unmeasured".to_owned(),
            level: "unmeasured".to_owned(),
            object: Some(output.describe(&u.object, qualify)),
            related: vec![],
            message: u.reason.clone(),
            evidence: vec![],
        }));
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

/// A finding about a whole population: no object is at fault.
fn rule_finding_entry(
    output: &CheckOutput,
    finding: &RuleFinding,
    qualify: bool,
    evidence: bool,
) -> Entry {
    Entry {
        section: Section::Findings,
        key: finding.rule_id.to_string(),
        level: severity(&finding.severity).to_owned(),
        object: None,
        related: finding
            .related
            .iter()
            .map(|id| output.describe(id, qualify))
            .collect(),
        message: finding.message.trim().to_owned(),
        evidence: if evidence {
            finding.evidence.iter().map(|e| e.locator.clone()).collect()
        } else {
            vec![]
        },
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
    Entry {
        section: Section::Findings,
        key: finding.rule_id.to_string(),
        level: severity(&finding.severity).to_owned(),
        object: Some(output.describe(&finding.object_id, qualify)),
        related: finding
            .related
            .iter()
            .map(|id| output.describe(id, qualify))
            .collect(),
        message: finding.message.trim().to_owned(),
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
    Entry {
        section: Section::NotEvaluated,
        key: outcome.rule_id.to_string(),
        level: reason(&outcome.reason).to_owned(),
        object: outcome
            .object_id
            .as_ref()
            .map(|id| output.describe(id, qualify)),
        related: vec![],
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
        let _ = writeln!(out, "\n    {}", entry.message);
        if !entry.related.is_empty() {
            let _ = writeln!(out, "    related: {}", entry.related.join("; "));
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
