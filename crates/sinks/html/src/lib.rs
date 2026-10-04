#![allow(clippy::doc_markdown)]

//! Self-contained HTML reports of a check run, from a template.
//!
//! An output sink, not a source adapter: it reads a finished [`Report`] and
//! the [`Project`] it was computed over, and depends on nothing else in the
//! engine. [`render`] fills a [`Template`] with the run's sections: a cover,
//! a summary, the rules, the findings' categories, the findings with their
//! locations and decisions, the not-evaluated outcomes, stale decisions and
//! the report tables. The result is one HTML file with its style inline and
//! no external resource, laid out for screen and print (A4 pages, one
//! section per page, table headers repeated), so a browser prints it as a
//! PDF.
//!
//! # Templates are data
//!
//! A template is HTML text with placeholders, `{{summary}}`, and nothing
//! else: no conditions, loops, expressions or includes. Rendering replaces
//! each placeholder with its [`Slot`]'s content and copies everything else
//! unchanged, so a template can reorder, wrap or leave out sections and
//! restyle them, but never run anything. The sink executes no code from a
//! template or a rule package. [`DEFAULT_TEMPLATE`] is built in; a custom
//! one is parsed with [`Template::parse`].
//!
//! Fail closed: a template must place [`Slot::Summary`] and
//! [`Slot::NotEvaluated`], so no report hides that the run was incomplete,
//! and an unknown or repeated section placeholder is refused rather than
//! left in the output.
//!
//! # Determinism
//!
//! The caller supplies the date ([`Options::date`]); nothing reads the
//! clock. Sections follow report order, rules and categories are sorted,
//! and every number is written with its shortest exact representation, so
//! identical input renders identical bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use axioval_ir::{
    DecisionStatus, EvidenceCheck, Finding, FindingDecision, Location, NotEvaluatedReason,
    ObjectId, Project, Report, ReportColumnKind, ReportTable, ReportValue, RuleStatus, Scope,
    Severity,
};
use thiserror::Error;

/// The built-in template: every section, in the order of [`Slot::SECTIONS`].
pub const DEFAULT_TEMPLATE: &str = include_str!("templates/report.html");

/// The built-in style sheet the [`Slot::Style`] placeholder inserts.
pub const DEFAULT_STYLE: &str = include_str!("templates/report.css");

/// What a placeholder is replaced with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Slot {
    /// `{{title}}`: [`Options::title`], escaped.
    Title,
    /// `{{date}}`: [`Options::date`], escaped.
    Date,
    /// `{{style}}`: [`DEFAULT_STYLE`], for inside a `<style>` element.
    Style,
    /// `{{cover}}`: title, date, overall status and the sources checked.
    Cover,
    /// `{{summary}}`: counts of findings by severity, not-evaluated
    /// outcomes, tables and decisions.
    Summary,
    /// `{{rules}}`: one row per rule with its status and counts.
    Rules,
    /// `{{categories}}`: findings counted per rule and category.
    Categories,
    /// `{{findings}}`: every finding per rule, with its object, location
    /// and decision.
    Findings,
    /// `{{not-evaluated}}`: every not-evaluated outcome and why.
    NotEvaluated,
    /// `{{stale-decisions}}`: decisions naming no finding of this run.
    StaleDecisions,
    /// `{{tables}}`: every report table, such as each takeoff.
    Tables,
}

impl Slot {
    /// Every slot, in the order the default template places them.
    pub const ALL: [Self; 11] = [
        Self::Title,
        Self::Date,
        Self::Style,
        Self::Cover,
        Self::Summary,
        Self::Rules,
        Self::Categories,
        Self::Findings,
        Self::NotEvaluated,
        Self::StaleDecisions,
        Self::Tables,
    ];
    /// The sections: slots placed at most once, each its own `<section>`
    /// (the cover a `<header>`).
    pub const SECTIONS: [Self; 8] = [
        Self::Cover,
        Self::Summary,
        Self::Rules,
        Self::Categories,
        Self::Findings,
        Self::NotEvaluated,
        Self::StaleDecisions,
        Self::Tables,
    ];
    /// The sections every template must place.
    pub const REQUIRED: [Self; 2] = [Self::Summary, Self::NotEvaluated];

    /// The placeholder's name, as written between the braces.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Date => "date",
            Self::Style => "style",
            Self::Cover => "cover",
            Self::Summary => "summary",
            Self::Rules => "rules",
            Self::Categories => "categories",
            Self::Findings => "findings",
            Self::NotEvaluated => "not-evaluated",
            Self::StaleDecisions => "stale-decisions",
            Self::Tables => "tables",
        }
    }

    fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|slot| slot.name() == name)
    }

    fn is_section(self) -> bool {
        Self::SECTIONS.contains(&self)
    }
}

/// Why a template was refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TemplateError {
    /// `{{` without a closing `}}` on the same line.
    #[error("line {line}: `{{{{` is not closed by `}}}}`")]
    Unclosed {
        /// The line, from 1.
        line: usize,
    },
    /// A placeholder naming no [`Slot`].
    #[error("line {line}: unknown placeholder `{{{{{name}}}}}`; known: {known}")]
    UnknownSlot {
        /// The line, from 1.
        line: usize,
        /// The name between the braces.
        name: String,
        /// Every slot's name, comma-separated.
        known: String,
    },
    /// A section placed twice.
    #[error("line {line}: section `{{{{{name}}}}}` is placed twice", name = .slot.name())]
    Repeated {
        /// The line of the second placement, from 1.
        line: usize,
        /// The section.
        slot: Slot,
    },
    /// A required section the template does not place.
    #[error("the template does not place `{{{{{name}}}}}`, which every report must show", name = .0.name())]
    Missing(Slot),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    Text(String),
    Slot(Slot),
}

/// A parsed template: literal text and placeholders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    parts: Vec<Part>,
}

impl Default for Template {
    /// The parsed [`DEFAULT_TEMPLATE`].
    fn default() -> Self {
        Self::parse(DEFAULT_TEMPLATE).expect("the built-in template is valid")
    }
}

impl Template {
    /// Parses `text`: every `{{name}}` (spaces inside the braces allowed)
    /// must name a [`Slot`], each section at most once, and the
    /// [`Slot::REQUIRED`] sections must appear.
    ///
    /// # Errors
    ///
    /// [`TemplateError`] naming the line of the first problem.
    pub fn parse(text: &str) -> Result<Self, TemplateError> {
        let mut parts = Vec::new();
        let mut placed = BTreeSet::new();
        let mut rest = text;
        let mut line = 1;
        while let Some(open) = rest.find("{{") {
            let (literal, tail) = rest.split_at(open);
            line += literal.matches('\n').count();
            if !literal.is_empty() {
                parts.push(Part::Text(literal.to_owned()));
            }
            let tail = &tail[2..];
            let close = tail
                .find("}}")
                .filter(|close| !tail[..*close].contains('\n'))
                .ok_or(TemplateError::Unclosed { line })?;
            let name = tail[..close].trim();
            let slot = Slot::named(name).ok_or_else(|| TemplateError::UnknownSlot {
                line,
                name: name.to_owned(),
                known: Slot::ALL.map(Slot::name).join(", "),
            })?;
            if slot.is_section() && !placed.insert(slot) {
                return Err(TemplateError::Repeated { line, slot });
            }
            parts.push(Part::Slot(slot));
            rest = &tail[close + 2..];
        }
        if !rest.is_empty() {
            parts.push(Part::Text(rest.to_owned()));
        }
        if let Some(missing) = Slot::REQUIRED
            .into_iter()
            .find(|slot| !placed.contains(slot))
        {
            return Err(TemplateError::Missing(missing));
        }
        Ok(Self { parts })
    }

    /// The slots the template places, in order.
    pub fn slots(&self) -> impl Iterator<Item = Slot> + '_ {
        self.parts.iter().filter_map(|part| match part {
            Part::Slot(slot) => Some(*slot),
            Part::Text(_) => None,
        })
    }
}

/// What a report says about the run beyond its outcomes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// The report's title.
    pub title: String,
    /// When the report was made, as the host states it (an ISO 8601
    /// date-time, say). Written as given.
    pub date: String,
    /// An external identity scheme (such as the IFC GlobalId scheme the
    /// source adapter attaches): when set, every object is shown with its
    /// alias in it.
    pub external_id_scheme: Option<String>,
}

impl Options {
    /// Options titled `Check report`, dated `date`, without an external
    /// identity.
    #[must_use]
    pub fn new(date: impl Into<String>) -> Self {
        Self {
            title: "Check report".to_owned(),
            date: date.into(),
            external_id_scheme: None,
        }
    }
}

/// Renders `report` through `template` as one HTML document.
#[must_use]
pub fn render(
    report: &Report,
    project: &Project,
    template: &Template,
    options: &Options,
) -> String {
    let view = View {
        report,
        project,
        options,
    };
    let mut html = String::new();
    for part in &template.parts {
        match part {
            Part::Text(text) => html.push_str(text),
            Part::Slot(slot) => html.push_str(&view.slot(*slot)),
        }
    }
    html
}

/// Escapes text for an HTML element or a quoted attribute.
#[must_use]
pub fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            c => escaped.push(c),
        }
    }
    escaped
}

/// A number with its shortest exact representation: never rounded, so an
/// interval's two bounds never read as one value.
fn number(value: f64) -> String {
    format!("{value}")
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
        NotEvaluatedReason::MissingService => "missing service",
        NotEvaluatedReason::BackendUnavailable => "backend unavailable",
        NotEvaluatedReason::IncompleteEvidence => "incomplete evidence",
        NotEvaluatedReason::InvalidEvidence => "invalid evidence",
        NotEvaluatedReason::InvalidDeclaration => "invalid declaration",
        NotEvaluatedReason::ResourceLimit => "resource limit",
        NotEvaluatedReason::UnboundConcept => "unbound concept",
        NotEvaluatedReason::NotRecorded => "not recorded",
    }
}

fn rule_status(status: RuleStatus) -> &'static str {
    match status {
        RuleStatus::Passed => "passed",
        RuleStatus::Failed => "failed",
        RuleStatus::NotEvaluated => "not evaluated",
        RuleStatus::NothingSelected => "nothing selected",
        RuleStatus::Skipped => "skipped",
    }
}

/// An HTML fragment id from a rule id.
fn anchor(prefix: &str, rule: &str) -> String {
    let id: String = rule
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("{prefix}-{id}")
}

/// Per-rule counts.
#[derive(Default)]
struct RuleCounts {
    findings: usize,
    not_evaluated: usize,
    tables: usize,
}

struct View<'a> {
    report: &'a Report,
    project: &'a Project,
    options: &'a Options,
}

impl View<'_> {
    fn slot(&self, slot: Slot) -> String {
        match slot {
            Slot::Title => escape(&self.options.title),
            Slot::Date => escape(&self.options.date),
            Slot::Style => DEFAULT_STYLE.to_owned(),
            Slot::Cover => self.cover(),
            Slot::Summary => self.summary(),
            Slot::Rules => self.rules(),
            Slot::Categories => self.categories(),
            Slot::Findings => self.findings(),
            Slot::NotEvaluated => self.not_evaluated(),
            Slot::StaleDecisions => self.stale_decisions(),
            Slot::Tables => self.tables(),
        }
    }

    /// The overall status as the CLI's exit status states it: findings
    /// fail the run, anything not evaluated leaves it incomplete.
    fn status(&self) -> (&'static str, String) {
        let findings = self.report.findings().len();
        let open = self.report.not_evaluated().len();
        if findings > 0 {
            ("failed", format!("Not passed: {findings} finding(s)"))
        } else if open > 0 {
            (
                "incomplete",
                format!("Incomplete: nothing found, but {open} outcome(s) were not evaluated"),
            )
        } else {
            (
                "passed",
                "Passed: everything was evaluated and nothing was found".to_owned(),
            )
        }
    }

    fn cover(&self) -> String {
        let (class, status) = self.status();
        let sources: BTreeSet<String> = self
            .project
            .objects()
            .map(|object| object.id.source.to_string())
            .chain(
                self.report
                    .findings()
                    .iter()
                    .filter_map(|finding| finding.scope.source())
                    .chain(
                        self.report
                            .not_evaluated()
                            .iter()
                            .filter_map(|outcome| outcome.scope.source()),
                    )
                    .map(ToString::to_string),
            )
            .collect();
        let mut html = format!(
            "<header class=\"cover\" id=\"cover\">\n<h1>{}</h1>\n<p class=\"date\">{}</p>\n<p class=\"status status-{class}\">{}</p>\n<dl>\n<dt>Sources</dt>\n<dd>",
            escape(&self.options.title),
            escape(&self.options.date),
            escape(&status),
        );
        if sources.is_empty() {
            html.push_str("none");
        } else {
            html.push_str(
                &sources
                    .iter()
                    .map(|source| format!("<code>{}</code>", escape(source)))
                    .collect::<Vec<_>>()
                    .join("<br>"),
            );
        }
        html.push_str("</dd>\n</dl>\n</header>\n");
        html
    }

    fn summary(&self) -> String {
        let findings = self.report.findings();
        let by = |wanted: Severity| {
            findings
                .iter()
                .filter(|finding| finding.severity == wanted)
                .count()
        };
        let decided = |wanted: Option<DecisionStatus>| {
            findings
                .iter()
                .filter(|finding| finding.decision.as_ref().map(|d| d.status) == wanted)
                .count()
        };
        let rows: [(&str, usize); 11] = [
            ("Findings", findings.len()),
            ("Errors", by(Severity::Error)),
            ("Warnings", by(Severity::Warning)),
            ("Information", by(Severity::Info)),
            ("Not evaluated", self.report.not_evaluated().len()),
            ("Rules with outcomes or tables", self.rule_counts().len()),
            ("Report tables", self.report.tables().len()),
            ("Findings accepted", decided(Some(DecisionStatus::Accepted))),
            ("Findings rejected", decided(Some(DecisionStatus::Rejected))),
            (
                "Findings open or undecided",
                decided(Some(DecisionStatus::Open)) + decided(None),
            ),
            ("Stale decisions", self.report.stale_decisions().len()),
        ];
        let (class, status) = self.status();
        let mut html = format!(
            "<section id=\"summary\">\n<h2>Summary</h2>\n<p class=\"status status-{class}\">{}</p>\n<table class=\"counts\">\n<tbody>\n",
            escape(&status)
        );
        for (label, count) in rows {
            let _ = writeln!(
                html,
                "<tr><th>{label}</th><td class=\"number\">{count}</td></tr>"
            );
        }
        html.push_str("</tbody>\n</table>\n</section>\n");
        html
    }

    fn rule_counts(&self) -> BTreeMap<String, RuleCounts> {
        let mut rules: BTreeMap<String, RuleCounts> = BTreeMap::new();
        for finding in self.report.findings() {
            rules
                .entry(finding.rule_id.to_string())
                .or_default()
                .findings += 1;
        }
        for outcome in self.report.not_evaluated() {
            rules
                .entry(outcome.rule_id.to_string())
                .or_default()
                .not_evaluated += 1;
        }
        for table in self.report.tables() {
            rules.entry(table.rule_id().to_string()).or_default().tables += 1;
        }
        for summary in self.report.rules() {
            rules.entry(summary.rule_id.to_string()).or_default();
        }
        rules
    }

    fn rules(&self) -> String {
        let summaries: BTreeMap<String, _> = self
            .report
            .rules()
            .iter()
            .map(|summary| (summary.rule_id.to_string(), summary))
            .collect();
        let counts = self.rule_counts();
        let mut html = String::from("<section id=\"rules\">\n<h2>Rules</h2>\n");
        if counts.is_empty() {
            html.push_str(
                "<p class=\"empty\">No rule reported an outcome or a table.</p>\n</section>\n",
            );
            return html;
        }
        html.push_str("<table>\n<thead><tr><th>Rule</th><th>Status</th><th class=\"number\">Checked</th><th class=\"number\">Findings</th><th class=\"number\">Not evaluated</th><th class=\"number\">Tables</th></tr></thead>\n<tbody>\n");
        for (rule, count) in &counts {
            let summary = summaries.get(rule);
            let status = match summary {
                Some(summary) => rule_status(summary.status),
                None if count.findings > 0 => "failed",
                None if count.not_evaluated > 0 => "not evaluated",
                None => "reported a table",
            };
            let checked =
                summary.map_or_else(|| "–".to_owned(), |summary| summary.checked.to_string());
            let _ = writeln!(
                html,
                "<tr><td><code>{}</code></td><td>{status}</td><td class=\"number\">{checked}</td><td class=\"number\">{}</td><td class=\"number\">{}</td><td class=\"number\">{}</td></tr>",
                escape(rule),
                count.findings,
                count.not_evaluated,
                count.tables,
            );
        }
        html.push_str("</tbody>\n</table>\n</section>\n");
        html
    }

    fn categories(&self) -> String {
        let mut counts: BTreeMap<(String, Vec<String>), usize> = BTreeMap::new();
        for finding in self.report.findings() {
            if !finding.categories.is_empty() {
                *counts
                    .entry((finding.rule_id.to_string(), finding.categories.clone()))
                    .or_default() += 1;
            }
        }
        let mut html = String::from("<section id=\"categories\">\n<h2>Categories</h2>\n");
        if counts.is_empty() {
            html.push_str("<p class=\"empty\">No finding is categorised.</p>\n</section>\n");
            return html;
        }
        html.push_str("<table>\n<thead><tr><th>Rule</th><th>Category</th><th class=\"number\">Findings</th></tr></thead>\n<tbody>\n");
        for ((rule, categories), count) in &counts {
            let _ = writeln!(
                html,
                "<tr><td><code>{}</code></td><td>{}</td><td class=\"number\">{count}</td></tr>",
                escape(rule),
                escape(&categories.join(" / ")),
            );
        }
        html.push_str("</tbody>\n</table>\n</section>\n");
        html
    }

    /// The scope, with the kind and alias of its object when it names one.
    fn subject(&self, scope: &Scope) -> String {
        let mut html = format!("<code>{}</code>", escape(&scope.to_string()));
        if let Some(object) = scope
            .object()
            .and_then(|id| self.report.object(self.project, id))
        {
            let _ = write!(
                html,
                "<br><span class=\"kind\">{}</span>",
                escape(&object.kind)
            );
            if let Some(alias) = self
                .options
                .external_id_scheme
                .as_deref()
                .and_then(|scheme| object.external_id(scheme))
            {
                let _ = write!(html, " <code>{}</code>", escape(alias));
            }
        }
        html
    }

    fn objects(&self, ids: &[ObjectId]) -> String {
        ids.iter()
            .map(|id| self.subject(&Scope::Object(id.clone())))
            .collect::<Vec<_>>()
            .join("<br>")
    }

    fn findings(&self) -> String {
        let mut html = String::from("<section id=\"findings\">\n<h2>Findings</h2>\n");
        if self.report.findings().is_empty() {
            html.push_str("<p class=\"empty\">No finding.</p>\n</section>\n");
            return html;
        }
        let mut by_rule: BTreeMap<String, Vec<(usize, &Finding)>> = BTreeMap::new();
        for (index, finding) in self.report.findings().iter().enumerate() {
            by_rule
                .entry(finding.rule_id.to_string())
                .or_default()
                .push((index + 1, finding));
        }
        for (rule, findings) in &by_rule {
            let _ = writeln!(
                html,
                "<h3 id=\"{}\">Rule <code>{}</code></h3>",
                anchor("findings", rule),
                escape(rule)
            );
            html.push_str("<table>\n<thead><tr><th>#</th><th>Severity</th><th>Object</th><th>Message</th><th>Related</th><th>Location</th><th>Decision</th></tr></thead>\n<tbody>\n");
            for (number, finding) in findings {
                let mut message = String::new();
                if !finding.categories.is_empty() {
                    let _ = write!(
                        message,
                        "<span class=\"muted\">{}</span><br>",
                        escape(&finding.categories.join(" / "))
                    );
                }
                message.push_str(&escape(&finding.message));
                message.push_str(&explanation(finding.explanation.as_ref()));
                let id = finding
                    .id
                    .map(|id| format!("<br><code class=\"muted\">{id}</code>"))
                    .unwrap_or_default();
                let severity = severity(&finding.severity);
                let _ = writeln!(
                    html,
                    "<tr><td>{number}{id}</td><td class=\"severity-{severity}\">{severity}</td><td>{}</td><td>{message}</td><td>{}</td><td>{}</td><td class=\"decision\">{}</td></tr>",
                    self.subject(&finding.scope),
                    self.objects(&finding.related),
                    location(finding.location.as_ref()),
                    decision(finding.decision.as_ref()),
                );
            }
            html.push_str("</tbody>\n</table>\n");
        }
        html.push_str("</section>\n");
        html
    }

    fn not_evaluated(&self) -> String {
        let mut html = String::from("<section id=\"not-evaluated\">\n<h2>Not evaluated</h2>\n");
        if self.report.not_evaluated().is_empty() {
            html.push_str("<p class=\"empty\">Every outcome was evaluated.</p>\n</section>\n");
            return html;
        }
        html.push_str("<p class=\"legend\">These were neither passed nor found: the run did not decide them, and the model has not passed them.</p>\n");
        html.push_str("<table>\n<thead><tr><th>Rule</th><th>Object</th><th>Reason</th><th>Message</th><th>Location</th></tr></thead>\n<tbody>\n");
        for outcome in self.report.not_evaluated() {
            let _ = writeln!(
                html,
                "<tr><td><code>{}</code></td><td>{}</td><td>{}</td><td>{}{}</td><td>{}</td></tr>",
                escape(&outcome.rule_id.to_string()),
                self.subject(&outcome.scope),
                reason(&outcome.reason),
                escape(&outcome.message),
                explanation(outcome.explanation.as_ref()),
                location(outcome.location.as_ref()),
            );
        }
        html.push_str("</tbody>\n</table>\n</section>\n");
        html
    }

    fn stale_decisions(&self) -> String {
        let mut html = String::from("<section id=\"stale-decisions\">\n<h2>Stale decisions</h2>\n");
        let stale = self.report.stale_decisions();
        if stale.is_empty() {
            html.push_str(
                "<p class=\"empty\">Every decision carried over names a finding of this run.</p>\n</section>\n",
            );
            return html;
        }
        html.push_str("<table>\n<thead><tr><th>Finding</th><th>Decision</th><th>By</th><th>At</th></tr></thead>\n<tbody>\n");
        for decision in stale {
            let _ = writeln!(
                html,
                "<tr><td><code>{}</code></td><td>{}</td><td>{}</td><td>{}</td></tr>",
                decision.finding,
                decision.status.as_str(),
                escape(&decision.author),
                decision.date,
            );
        }
        html.push_str("</tbody>\n</table>\n</section>\n");
        html
    }

    fn tables(&self) -> String {
        let mut html = String::from("<section id=\"tables\">\n<h2>Tables</h2>\n");
        if self.report.tables().is_empty() {
            html.push_str("<p class=\"empty\">No rule reported a table.</p>\n</section>\n");
            return html;
        }
        html.push_str("<p class=\"legend\">A shaded value <em>a – b</em> is known only to lie between its bounds; <em>not evaluated</em> was not measured, and the rule's not-evaluated outcomes say why.</p>\n");
        for table in self.report.tables() {
            self.table(&mut html, table);
        }
        html.push_str("</section>\n");
        html
    }

    fn table(&self, html: &mut String, table: &ReportTable) {
        let rule = table.rule_id().to_string();
        let _ = writeln!(
            html,
            "<h3 id=\"{}-{}\"><code>{}</code> · {}</h3>",
            anchor("table", &rule),
            escape(table.name()),
            escape(&rule),
            escape(table.name()),
        );
        html.push_str("<table>\n<thead><tr><th>Scope</th>");
        for group in table.group_by() {
            let _ = write!(html, "<th>{}</th>", escape(group));
        }
        for column in table.columns() {
            let unit = column
                .unit_symbol()
                .map(|unit| format!(" [{}]", escape(&unit)))
                .unwrap_or_default();
            let class = if column.kind == ReportColumnKind::Text {
                ""
            } else {
                " class=\"number\""
            };
            let _ = write!(html, "<th{class}>{}{unit}</th>", escape(&column.id));
        }
        html.push_str("</tr></thead>\n<tbody>\n");
        for row in table.rows() {
            let _ = write!(html, "<tr><td>{}</td>", self.subject(row.scope()));
            for group in row.group() {
                let _ = write!(html, "<td>{}</td>", escape(group));
            }
            for value in row.values() {
                html.push_str(&cell(value));
            }
            html.push_str("</tr>\n");
        }
        html.push_str("</tbody>\n</table>\n");
    }
}

/// One table cell: an exact number, an interval as both its bounds, text,
/// or `not evaluated`.
fn cell(value: &ReportValue) -> String {
    match value {
        ReportValue::Exact { value } => {
            format!("<td class=\"number exact\">{}</td>", number(*value))
        }
        ReportValue::Interval { lower, upper } => format!(
            "<td class=\"number bounded\">{} – {}</td>",
            number(*lower),
            number(*upper)
        ),
        ReportValue::Text { value } => format!("<td>{}</td>", escape(value)),
        ReportValue::Unknown => "<td class=\"unknown\">not evaluated</td>".to_owned(),
    }
}

/// Storeys and spaces, each by name else by id, and why the location is
/// incomplete.
fn location(location: Option<&Location>) -> String {
    let Some(location) = location else {
        return String::new();
    };
    let mut lines = Vec::new();
    for (label, places) in [("Storey", &location.storeys), ("Space", &location.spaces)] {
        for place in places {
            let name = place.name.clone().unwrap_or_else(|| place.id.to_string());
            lines.push(format!("{label}: {}", escape(&name)));
        }
    }
    if let Some(unresolved) = &location.unresolved {
        lines.push(format!(
            "<span class=\"muted\">unresolved: {}</span>",
            escape(unresolved)
        ));
    }
    lines.join("<br>")
}

/// A finding's decision: status, by whom and when, assignment, and its
/// thread.
fn decision(decision: Option<&FindingDecision>) -> String {
    let Some(decision) = decision else {
        return "<span class=\"muted\">undecided</span>".to_owned();
    };
    let mut html = format!(
        "<p><strong>{}</strong> by {}, {}</p>",
        decision.status.as_str(),
        escape(&decision.author),
        decision.date
    );
    if decision.evidence == EvidenceCheck::Changed {
        html.push_str("<p class=\"changed\">changed since the decision</p>");
    }
    let mut facts = Vec::new();
    if let Some(assignee) = &decision.assigned_to {
        facts.push(format!("assigned to {}", escape(assignee)));
    }
    if let Some(due) = &decision.due_date {
        facts.push(format!("due {due}"));
    }
    if let Some(priority) = &decision.priority {
        facts.push(format!("priority {}", escape(priority)));
    }
    if !decision.labels.is_empty() {
        facts.push(format!("labels {}", escape(&decision.labels.join(", "))));
    }
    if !facts.is_empty() {
        let _ = write!(html, "<p class=\"muted\">{}</p>", facts.join("; "));
    }
    for comment in &decision.comments {
        let _ = write!(
            html,
            "<p>{} ({}): {}</p>",
            escape(&comment.author),
            comment.date,
            escape(&comment.text)
        );
    }
    html
}

/// The deciding path of an expression rule's explanation, as a list under
/// the message; nothing for any other rule.
fn explanation(explanation: Option<&axioval_ir::Explanation>) -> String {
    let Some(explanation) = explanation else {
        return String::new();
    };
    let mut html = String::from("<ol class=\"why\">");
    for step in explanation.deciding() {
        let _ = write!(html, "<li>{}</li>", escape(&step.describe()));
    }
    html.push_str("</ol>");
    html
}
