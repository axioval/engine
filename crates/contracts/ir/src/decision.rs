//! Reviewers' decisions about findings, kept across re-checks.
//!
//! A reviewer accepts or rejects a finding, with a comment. The decision is
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

/// One reviewer's decision about one finding.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    /// The identity of the finding decided about.
    pub finding: FindingId,
    pub status: DecisionStatus,
    /// Who decided; never blank.
    pub author: String,
    /// When.
    pub date: DateTime,
    /// Why; may be empty, and is then absent on the wire.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    /// The finding as it was decided, when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<DecisionBasis>,
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
            comment: String::new(),
            basis: None,
        })
    }

    /// The same decision with a comment.
    #[must_use]
    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = comment.into();
        self
    }

    /// The same decision recording `finding` as its basis.
    #[must_use]
    pub fn with_basis(mut self, finding: &Finding) -> Self {
        self.basis = Some(DecisionBasis::of(finding));
        self
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
    /// [`DecisionError::Duplicate`] when two name one finding, and
    /// [`DecisionError::BlankAuthor`] when one has a blank author.
    pub fn new(decisions: impl IntoIterator<Item = Decision>) -> Result<Self, DecisionError> {
        let mut decisions: Vec<Decision> = decisions.into_iter().collect();
        decisions.sort_by_key(|decision| decision.finding);
        for pair in decisions.windows(2) {
            if pair[0].finding == pair[1].finding {
                return Err(DecisionError::Duplicate(pair[0].finding));
            }
        }
        if let Some(blank) = decisions.iter().find(|d| d.author.trim().is_empty()) {
            return Err(DecisionError::BlankAuthor(blank.finding));
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
    /// [`DecisionError::BlankAuthor`] when its author is blank.
    pub fn record(&mut self, decision: Decision) -> Result<Option<Decision>, DecisionError> {
        if decision.author.trim().is_empty() {
            return Err(DecisionError::BlankAuthor(decision.finding));
        }
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
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FindingDecision {
    pub status: DecisionStatus,
    pub author: String,
    pub date: DateTime,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    /// Whether the finding changed since it was decided.
    pub evidence: EvidenceCheck,
    /// What changed, when [`EvidenceCheck::Changed`]; empty otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<DecisionChange>,
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
            comment: decision.comment.clone(),
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
