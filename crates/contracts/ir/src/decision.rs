//! Reviewers' decisions about findings, kept across re-checks.
//!
//! A reviewer accepts or rejects a finding, with comments, and may assign
//! it to someone, set a due date and a priority, and label it. The decision is
//! keyed by the finding's [`FindingId`], so checking a revised model carries
//! it over to the finding with the same identity. A decision whose finding is
//! gone is *stale* and listed apart; a decision whose finding is still there
//! but whose evidence changed is flagged, where the change can be told.
//!
//! Decisions only mark findings. They never remove a finding, never change
//! its severity, and never touch a not-evaluated outcome: an outcome that
//! could not be evaluated is not a finding anybody decided.
//!
//! # What counts as changed
//!
//! A decision may record its [`DecisionBasis`]: the rule, message, severity
//! and the number of evidence items, exact and inexact, of the finding when
//! it was decided. A re-check compares those with the finding now:
//!
//! - rule and message are part of the identity, so they differ only for a
//!   decision recorded against something else;
//! - the severity changes when a revised model is graded into another band
//!   or another override holds;
//! - the evidence count changes when the finding is decided by more or fewer
//!   facts, and the inexact count when facts become approximate or exact.
//!
//! Evidence *locators* are not compared: a source numbers its entities anew
//! on every export, so a locator changes with no change to the model, and
//! comparing them would flag every decision after a re-export. A changed
//! measured value is not missed either: it is written into the message, so
//! the finding gets a new identity and the old decision becomes stale.
//! Without a basis nothing can be compared, and the decision says so
//! ([`EvidenceCheck::Unknown`]) instead of claiming the evidence unchanged.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{DateTime, Finding, FindingId, Report, RuleId, Severity};

/// What the reviewer decided.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    /// Seen, not decided yet.
    Open,
    /// The finding stands and the reviewer accepts it.
    Accepted,
    /// The reviewer rejects the finding.
    Rejected,
}

impl DecisionStatus {
    /// `open`, `accepted` or `rejected`, as written on the wire.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }
}

/// What a finding looked like when it was decided.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionBasis {
    pub rule_id: RuleId,
    pub message: String,
    pub severity: Severity,
    /// How many evidence items the finding cited.
    pub evidence: usize,
    /// How many of them were inexact.
    pub inexact_evidence: usize,
}

impl DecisionBasis {
    /// The basis of a decision about `finding` as it is now.
    #[must_use]
    pub fn of(finding: &Finding) -> Self {
        Self {
            rule_id: finding.rule_id.clone(),
            message: finding.message.clone(),
            severity: finding.severity.clone(),
            evidence: finding.evidence.len(),
            inexact_evidence: finding.evidence.iter().filter(|e| !e.exact).count(),
        }
    }

    /// What differs between this basis and `finding` now, in facet order.
    #[must_use]
    pub fn changes(&self, finding: &Finding) -> Vec<DecisionChange> {
        let now = Self::of(finding);
        let mut changes = Vec::new();
        let mut differ = |facet, decided: String, now: String| {
            if decided != now {
                changes.push(DecisionChange {
                    facet,
                    decided,
                    now,
                });
            }
        };
        differ(
            ChangedFacet::Rule,
            self.rule_id.to_string(),
            now.rule_id.to_string(),
        );
        differ(ChangedFacet::Message, self.message.clone(), now.message);
        differ(
            ChangedFacet::Severity,
            severity(&self.severity).to_owned(),
            severity(&now.severity).to_owned(),
        );
        differ(
            ChangedFacet::Evidence,
            self.evidence.to_string(),
            now.evidence.to_string(),
        );
        differ(
            ChangedFacet::InexactEvidence,
            self.inexact_evidence.to_string(),
            now.inexact_evidence.to_string(),
        );
        changes
    }
}

fn severity(severity: &Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

/// One comment of a decision's thread: who said what, when.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionComment {
    /// Who; never blank.
    pub author: String,
    /// When.
    pub date: DateTime,
    /// What.
    pub text: String,
    /// The identity an issue tracker gave the comment (a BCF comment GUID),
    /// kept so writing it back updates it instead of adding a copy. Absent
    /// for a comment made here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Uuid>,
}

impl DecisionComment {
    /// A comment made here, by `author` at `date`.
    pub fn new(author: impl Into<String>, date: DateTime, text: impl Into<String>) -> Self {
        Self {
            author: author.into(),
            date,
            text: text.into(),
            id: None,
        }
    }

    /// The same comment under an issue tracker's identity.
    #[must_use]
    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = Some(id);
        self
    }
}

/// A thread as written: one comment by the decision's own author at its
/// date and without an identity is the single `comment` it always was;
/// any other thread is `comments`.
fn thread_to_wire(
    comments: Vec<DecisionComment>,
    author: &str,
    date: DateTime,
) -> (String, Vec<DecisionComment>) {
    match comments.as_slice() {
        [only] if only.author == author && only.date == date && only.id.is_none() => {
            (only.text.clone(), Vec::new())
        }
        _ => (String::new(), comments),
    }
}

/// A thread as read: `comment` is one comment by the decision's author at
/// its date, `comments` the whole thread; never both.
fn thread_from_wire(
    comment: String,
    comments: Vec<DecisionComment>,
    author: &str,
    date: DateTime,
) -> Result<Vec<DecisionComment>, String> {
    match (comment.is_empty(), comments.is_empty()) {
        (true, _) => Ok(comments),
        (false, true) => Ok(vec![DecisionComment::new(author, date, comment)]),
        (false, false) => Err("a decision has `comment` or `comments`, never both".to_owned()),
    }
}

/// One reviewer's decision about one finding.
///
/// On the wire a thread of one comment by the decision's author at its
/// date is the field `comment`, as before threads existed; any other thread
/// is `comments`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "DecisionWire", into = "DecisionWire")]
pub struct Decision {
    /// The identity of the finding decided about.
    pub finding: FindingId,
    pub status: DecisionStatus,
    /// Who decided; never blank.
    pub author: String,
    /// When.
    pub date: DateTime,
    /// Why, and what was said since, oldest first; may be empty.
    pub comments: Vec<DecisionComment>,
    /// Who the finding is assigned to; never blank.
    pub assigned_to: Option<String>,
    /// When the finding is due to be dealt with.
    pub due_date: Option<DateTime>,
    /// How urgent, in the project's own vocabulary (`High`, `Normal`, ...);
    /// never blank.
    pub priority: Option<String>,
    /// Labels the reviewer gave the finding, in order; none blank.
    pub labels: Vec<String>,
    /// The finding as it was decided, when recorded.
    pub basis: Option<DecisionBasis>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionWire {
    finding: FindingId,
    status: DecisionStatus,
    author: String,
    date: DateTime,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    comment: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    comments: Vec<DecisionComment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    assigned_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    due_date: Option<DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    priority: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    basis: Option<DecisionBasis>,
}

impl TryFrom<DecisionWire> for Decision {
    type Error = String;
    fn try_from(wire: DecisionWire) -> Result<Self, String> {
        Ok(Self {
            comments: thread_from_wire(wire.comment, wire.comments, &wire.author, wire.date)?,
            finding: wire.finding,
            status: wire.status,
            author: wire.author,
            date: wire.date,
            assigned_to: wire.assigned_to,
            due_date: wire.due_date,
            priority: wire.priority,
            labels: wire.labels,
            basis: wire.basis,
        })
    }
}

impl From<Decision> for DecisionWire {
    fn from(decision: Decision) -> Self {
        let (comment, comments) =
            thread_to_wire(decision.comments, &decision.author, decision.date);
        Self {
            finding: decision.finding,
            status: decision.status,
            author: decision.author,
            date: decision.date,
            comment,
            comments,
            assigned_to: decision.assigned_to,
            due_date: decision.due_date,
            priority: decision.priority,
            labels: decision.labels,
            basis: decision.basis,
        }
    }
}

impl Decision {
    /// A decision without a comment or basis.
    ///
    /// # Errors
    ///
    /// [`DecisionError::BlankAuthor`] when `author` is blank.
    pub fn new(
        finding: FindingId,
        status: DecisionStatus,
        author: impl Into<String>,
        date: DateTime,
    ) -> Result<Self, DecisionError> {
        let author = author.into();
        if author.trim().is_empty() {
            return Err(DecisionError::BlankAuthor(finding));
        }
        Ok(Self {
            finding,
            status,
            author,
            date,
            comments: Vec::new(),
            assigned_to: None,
            due_date: None,
            priority: None,
            labels: Vec::new(),
            basis: None,
        })
    }

    /// The same decision with its thread replaced by `comment`, by its
    /// author at its date; an empty `comment` leaves the thread empty.
    #[must_use]
    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        let comment = comment.into();
        self.comments = if comment.is_empty() {
            Vec::new()
        } else {
            vec![DecisionComment::new(
                self.author.clone(),
                self.date,
                comment,
            )]
        };
        self
    }

    /// The same decision with `comment` added to the end of its thread.
    #[must_use]
    pub fn with_reply(mut self, comment: DecisionComment) -> Self {
        self.comments.push(comment);
        self
    }

    /// The same decision recording `finding` as its basis.
    #[must_use]
    pub fn with_basis(mut self, finding: &Finding) -> Self {
        self.basis = Some(DecisionBasis::of(finding));
        self
    }

    /// The same decision assigning the finding to `assignee`.
    #[must_use]
    pub fn with_assignee(mut self, assignee: impl Into<String>) -> Self {
        self.assigned_to = Some(assignee.into());
        self
    }

    /// The same decision due at `date`.
    #[must_use]
    pub fn with_due_date(mut self, date: DateTime) -> Self {
        self.due_date = Some(date);
        self
    }

    /// The same decision with a priority.
    #[must_use]
    pub fn with_priority(mut self, priority: impl Into<String>) -> Self {
        self.priority = Some(priority.into());
        self
    }

    /// The same decision with labels, in order.
    #[must_use]
    pub fn with_labels(mut self, labels: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.labels = labels.into_iter().map(Into::into).collect();
        self
    }

    /// Refuses a blank author, assignee, priority, label or comment author.
    fn validate(&self) -> Result<(), DecisionError> {
        if self.author.trim().is_empty() {
            return Err(DecisionError::BlankAuthor(self.finding));
        }
        let blank = |field| {
            Err(DecisionError::Blank {
                finding: self.finding,
                field,
            })
        };
        if self
            .assigned_to
            .as_deref()
            .is_some_and(|a| a.trim().is_empty())
        {
            return blank("assigned_to");
        }
        if self
            .priority
            .as_deref()
            .is_some_and(|p| p.trim().is_empty())
        {
            return blank("priority");
        }
        if self.labels.iter().any(|label| label.trim().is_empty()) {
            return blank("labels");
        }
        if self.comments.iter().any(|c| c.author.trim().is_empty()) {
            return blank("comment author");
        }
        Ok(())
    }
}

/// Why decisions were refused.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DecisionError {
    /// Two decisions name one finding, so which holds is ambiguous.
    #[error("two decisions name finding {0}")]
    Duplicate(FindingId),
    /// A decision names nobody who made it.
    #[error("the decision about finding {0} has a blank author")]
    BlankAuthor(FindingId),
    /// A decision's assignee, priority, a label or a comment's author is
    /// blank.
    #[error("the decision about finding {finding} has a blank {field}")]
    Blank {
        /// The finding decided about.
        finding: FindingId,
        /// `assigned_to`, `priority`, `labels` or `comment author`.
        field: &'static str,
    },
    /// A finding of the report has no identity, so no decision can be
    /// matched to it; see [`Report::identify_findings`].
    #[error("finding of rule {0} has no identity; identify the report's findings first")]
    Unidentified(RuleId),
}

/// A set of decisions, at most one per finding, ordered by finding identity.
///
/// On the wire an object with one field, `decisions`, so a file can gain
/// fields later without breaking readers of this one.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "DecisionsWire", into = "DecisionsWire")]
pub struct Decisions {
    decisions: Vec<Decision>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionsWire {
    decisions: Vec<Decision>,
}

impl TryFrom<DecisionsWire> for Decisions {
    type Error = DecisionError;
    fn try_from(wire: DecisionsWire) -> Result<Self, DecisionError> {
        Self::new(wire.decisions)
    }
}

impl From<Decisions> for DecisionsWire {
    fn from(decisions: Decisions) -> Self {
        Self {
            decisions: decisions.decisions,
        }
    }
}

impl Decisions {
    /// The decisions, ordered by finding.
    ///
    /// # Errors
    ///
    /// [`DecisionError::Duplicate`] when two name one finding,
    /// [`DecisionError::BlankAuthor`] when one has a blank author, and
    /// [`DecisionError::Blank`] when one has a blank assignee, priority or
    /// label.
    pub fn new(decisions: impl IntoIterator<Item = Decision>) -> Result<Self, DecisionError> {
        let mut decisions: Vec<Decision> = decisions.into_iter().collect();
        decisions.sort_by_key(|decision| decision.finding);
        for pair in decisions.windows(2) {
            if pair[0].finding == pair[1].finding {
                return Err(DecisionError::Duplicate(pair[0].finding));
            }
        }
        for decision in &decisions {
            decision.validate()?;
        }
        Ok(Self { decisions })
    }

    /// Every decision, ordered by finding.
    #[must_use]
    pub fn decisions(&self) -> &[Decision] {
        &self.decisions
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.decisions.is_empty()
    }

    /// The decision about `finding`, if any.
    #[must_use]
    pub fn get(&self, finding: FindingId) -> Option<&Decision> {
        self.decisions
            .binary_search_by_key(&finding, |decision| decision.finding)
            .ok()
            .map(|at| &self.decisions[at])
    }

    /// Records `decision`, replacing any earlier one about its finding, and
    /// returns the replaced one.
    ///
    /// # Errors
    ///
    /// [`DecisionError::BlankAuthor`] when its author is blank, and
    /// [`DecisionError::Blank`] when its assignee, priority or a label is.
    pub fn record(&mut self, decision: Decision) -> Result<Option<Decision>, DecisionError> {
        decision.validate()?;
        match self
            .decisions
            .binary_search_by_key(&decision.finding, |existing| existing.finding)
        {
            Ok(at) => Ok(Some(std::mem::replace(&mut self.decisions[at], decision))),
            Err(at) => {
                self.decisions.insert(at, decision);
                Ok(None)
            }
        }
    }
}

/// Whether a finding's evidence changed since it was decided.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceCheck {
    /// The decision's basis matches the finding.
    Unchanged,
    /// Something the basis records differs; see the decision's changes.
    Changed,
    /// The decision records no basis, so nothing could be compared.
    Unknown,
}

/// What differs between a decision's basis and its finding now.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangedFacet {
    Rule,
    Message,
    Severity,
    /// The number of evidence items.
    Evidence,
    /// The number of inexact evidence items.
    InexactEvidence,
}

impl ChangedFacet {
    /// `rule`, `message`, `severity`, `evidence` or `inexact_evidence`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Message => "message",
            Self::Severity => "severity",
            Self::Evidence => "evidence",
            Self::InexactEvidence => "inexact_evidence",
        }
    }
}

/// One facet that differs, as decided and as now.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionChange {
    pub facet: ChangedFacet,
    pub decided: String,
    pub now: String,
}

/// The decision carried over to a finding of a report.
///
/// Its thread is written as [`Decision`]'s is: one comment by its author at
/// its date is `comment`, any other thread `comments`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "FindingDecisionWire", into = "FindingDecisionWire")]
pub struct FindingDecision {
    pub status: DecisionStatus,
    pub author: String,
    pub date: DateTime,
    /// The decision's thread, oldest first.
    pub comments: Vec<DecisionComment>,
    /// Who the finding is assigned to.
    pub assigned_to: Option<String>,
    /// When the finding is due.
    pub due_date: Option<DateTime>,
    /// How urgent, in the project's own vocabulary.
    pub priority: Option<String>,
    /// The reviewer's labels, in order.
    pub labels: Vec<String>,
    /// Whether the finding changed since it was decided.
    pub evidence: EvidenceCheck,
    /// What changed, when [`EvidenceCheck::Changed`]; empty otherwise.
    pub changes: Vec<DecisionChange>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FindingDecisionWire {
    status: DecisionStatus,
    author: String,
    date: DateTime,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    comment: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    comments: Vec<DecisionComment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    assigned_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    due_date: Option<DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    priority: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    labels: Vec<String>,
    evidence: EvidenceCheck,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    changes: Vec<DecisionChange>,
}

impl TryFrom<FindingDecisionWire> for FindingDecision {
    type Error = String;
    fn try_from(wire: FindingDecisionWire) -> Result<Self, String> {
        Ok(Self {
            comments: thread_from_wire(wire.comment, wire.comments, &wire.author, wire.date)?,
            status: wire.status,
            author: wire.author,
            date: wire.date,
            assigned_to: wire.assigned_to,
            due_date: wire.due_date,
            priority: wire.priority,
            labels: wire.labels,
            evidence: wire.evidence,
            changes: wire.changes,
        })
    }
}

impl From<FindingDecision> for FindingDecisionWire {
    fn from(decision: FindingDecision) -> Self {
        let (comment, comments) =
            thread_to_wire(decision.comments, &decision.author, decision.date);
        Self {
            status: decision.status,
            author: decision.author,
            date: decision.date,
            comment,
            comments,
            assigned_to: decision.assigned_to,
            due_date: decision.due_date,
            priority: decision.priority,
            labels: decision.labels,
            evidence: decision.evidence,
            changes: decision.changes,
        }
    }
}

impl FindingDecision {
    /// `decision` as it applies to `finding` now.
    #[must_use]
    pub fn carried(decision: &Decision, finding: &Finding) -> Self {
        let (evidence, changes) = match &decision.basis {
            None => (EvidenceCheck::Unknown, Vec::new()),
            Some(basis) => {
                let changes = basis.changes(finding);
                if changes.is_empty() {
                    (EvidenceCheck::Unchanged, changes)
                } else {
                    (EvidenceCheck::Changed, changes)
                }
            }
        };
        Self {
            status: decision.status,
            author: decision.author.clone(),
            date: decision.date,
            comments: decision.comments.clone(),
            assigned_to: decision.assigned_to.clone(),
            due_date: decision.due_date,
            priority: decision.priority.clone(),
            labels: decision.labels.clone(),
            evidence,
            changes,
        }
    }
}

impl Report {
    /// Carries `decisions` over to this report's findings.
    ///
    /// Each finding whose identity a decision names is marked with it
    /// ([`Finding::decision`]); every other finding is left undecided.
    /// Decisions naming no finding are listed in
    /// [`Report::stale_decisions`], by finding identity. Decisions applied
    /// before are replaced. Not-evaluated outcomes are never touched.
    ///
    /// # Errors
    ///
    /// [`DecisionError::Unidentified`] when there are decisions and a finding
    /// has no identity: a decision about it could not be told apart from a
    /// stale one. Nothing is changed then.
    pub fn apply_decisions(&mut self, decisions: &Decisions) -> Result<(), DecisionError> {
        if !decisions.is_empty()
            && let Some(unidentified) = self.findings.iter().find(|f| f.id.is_none())
        {
            return Err(DecisionError::Unidentified(unidentified.rule_id.clone()));
        }
        let mut matched = vec![false; decisions.decisions().len()];
        for finding in &mut self.findings {
            let at = finding.id.and_then(|id| {
                decisions
                    .decisions()
                    .binary_search_by_key(&id, |decision| decision.finding)
                    .ok()
            });
            finding.decision = at.map(|at| {
                matched[at] = true;
                FindingDecision::carried(&decisions.decisions()[at], finding)
            });
        }
        self.stale_decisions = decisions
            .decisions()
            .iter()
            .zip(matched)
            .filter(|(_, matched)| !matched)
            .map(|(decision, _)| decision.clone())
            .collect();
        Ok(())
    }
}
