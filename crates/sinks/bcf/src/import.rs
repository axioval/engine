//! Reading BCF topics back into review decisions.
//!
//! An archive this crate wrote names each finding's topic by the finding's
//! identity, so a reviewer's work in another BCF tool (a closed topic, a
//! comment) can be carried into the next check run as [`Decision`]s. See
//! [`import`] for the mapping.

use std::collections::BTreeMap;

use axioval_ir::{
    DateTime, Decision, DecisionComment, DecisionError, DecisionStatus, Decisions, Finding,
    FindingId, IdentityError, Project, Report, finding_ids, not_evaluated_ids,
};
use openbim_bcf::{BcfError, Comment, Limits, Markup};
use thiserror::Error;
use uuid::Uuid;

use crate::{IFC_GLOBAL_ID_SCHEME, exported_labels, priority};

/// Label prefixes the export writes itself (folder, category, storey,
/// space), never read as a reviewer's labels, even when the importing host
/// would not write them now.
const EXPORTED_LABEL_PREFIXES: [&str; 4] = ["Folder: ", "Category: ", "Storey: ", "Space: "];

/// Topic statuses read as an accepted finding, compared ignoring ASCII case:
/// a topic closed or resolved in another tool is a finding the reviewer
/// dealt with.
pub const ACCEPTED_STATUSES: [&str; 4] = ["Accepted", "Closed", "Resolved", "Done"];

/// Topic statuses read as a rejected finding, compared ignoring ASCII case.
pub const REJECTED_STATUSES: [&str; 1] = ["Rejected"];

/// How much an imported archive may decompress to: 32 MiB per entry, 256 MiB
/// in all, 100 000 entries. Tighter than the reader's defaults: an archive
/// of review topics is small, and it comes from other parties.
pub const IMPORT_LIMITS: Limits = Limits {
    max_total_uncompressed: 256 * 1024 * 1024,
    max_entry_uncompressed: 32 * 1024 * 1024,
    max_entries: 100_000,
};

/// Why an archive could not be imported. Nothing was imported then.
#[derive(Debug, Error)]
pub enum ImportError {
    /// The bytes are not a BCF archive the reader can read, or exceed
    /// [`IMPORT_LIMITS`].
    #[error("cannot read the BCF archive: {0}")]
    Read(#[from] BcfError),
    /// The report names an object the project does not contain, so the
    /// report was not computed over this project.
    #[error("report names {0}, which is not in the project")]
    UnknownObject(axioval_ir::ObjectId),
    /// Two topics carry one finding's GUID, so which one holds is
    /// ambiguous.
    #[error("{0}")]
    Decisions(#[from] DecisionError),
}

/// What an archive's topics say about a report's findings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Import {
    /// One decision per topic that names a finding of the report and
    /// carries review state, keyed by the finding's identity.
    pub decisions: Decisions,
    /// Every topic that yields no decision about a current finding, in
    /// archive order. Never dropped: a topic made by another tool, or about
    /// a finding that is gone, is listed here.
    pub unmatched: Vec<UnmatchedTopic>,
}

/// A topic that names no current finding, or whose review state cannot be
/// read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnmatchedTopic {
    /// The topic GUID as written, if any.
    pub guid: Option<String>,
    /// The topic title as written, if any.
    pub title: Option<String>,
    /// The topic status as written, if any.
    pub status: Option<String>,
    /// Why it yields no decision.
    pub reason: Unmatched,
}

/// Why a topic yields no decision about a current finding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unmatched {
    /// Its GUID is no finding of the report: the finding was fixed or
    /// changed into a new one, or another tool made the topic.
    NoFinding,
    /// Its GUID is a not-evaluated outcome's, which is never decided.
    NotEvaluated,
    /// It has no GUID, or one that is not a UUID.
    NoGuid,
    /// It names a finding, but its review state cannot be read, e.g. a date
    /// without a UTC offset or a comment without an author.
    Unreadable(String),
}

impl Unmatched {
    /// `no-finding`, `not-evaluated`, `no-guid` or `unreadable`.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NoFinding => "no-finding",
            Self::NotEvaluated => "not-evaluated",
            Self::NoGuid => "no-guid",
            Self::Unreadable(_) => "unreadable",
        }
    }
}

/// Reads the BCF 2.1 or 3.0 archive `bytes` and maps its topics onto
/// decisions about `report`'s findings, as [`import_topics`] does.
///
/// The archive is read within [`IMPORT_LIMITS`].
///
/// # Errors
///
/// [`ImportError::Read`] when it is not a readable BCF archive or exceeds
/// the limits, and as [`import_topics`].
pub fn import(
    bytes: &[u8],
    report: &Report,
    project: &Project,
    rule_labels: &BTreeMap<String, Vec<String>>,
) -> Result<Import, ImportError> {
    let archive = openbim_bcf::read_slice_with(bytes, IMPORT_LIMITS)?;
    import_topics(archive.topics(), report, project, rule_labels)
}

/// Maps BCF topics onto decisions about `report`'s findings.
///
/// A topic is matched to the finding whose identity over
/// [`IFC_GLOBAL_ID_SCHEME`] is its GUID, the identity the export wrote it
/// under. A matched topic yields a decision when it carries review state: a
/// status in [`ACCEPTED_STATUSES`] or [`REJECTED_STATUSES`] (any other
/// status is open), the comment the export wrote for a decision, any other
/// comment, an assignee, a due date, a priority other than the one the
/// finding's severity gives, or a label the export does not write. An
/// untouched topic yields none, so exporting and importing again decides
/// nothing.
///
/// - **Status** from the topic status, never from comment text: a status
///   changed in another tool wins.
/// - **Author and date** of the latest of the export's decision comment and
///   the topic's modification; without either, of its last comment, and
///   without comments, of its creation.
/// - **Comments**: the text of the export's decision comment without its
///   status word and change note, by its author at its date, then every
///   other comment in archive order, each keeping its GUID as
///   [`DecisionComment::id`] unless it is the one the export derives.
/// - **Assignee** and **due date** from `AssignedTo` and `DueDate`.
/// - **Priority** from `Priority` when it differs from the one the export
///   derives from the finding's severity.
/// - **Labels**: the topic's labels the export does not write for the
///   finding with `rule_labels` (the rule id, its rule labels, `Decision
///   changed`), nor any `Folder: `, `Category: `, `Storey: ` or `Space: `
///   label; pass the `rule_labels` the archive was exported with.
///
/// Every other topic is listed in [`Import::unmatched`], with why.
///
/// # Errors
///
/// [`ImportError::UnknownObject`] when the report names an object the
/// project does not contain, and [`ImportError::Decisions`] when two topics
/// carry one finding's GUID.
pub fn import_topics<'a>(
    topics: impl IntoIterator<Item = &'a Markup>,
    report: &Report,
    project: &Project,
    rule_labels: &BTreeMap<String, Vec<String>>,
) -> Result<Import, ImportError> {
    let findings: BTreeMap<Uuid, (FindingId, &Finding)> =
        finding_ids(report, project, IFC_GLOBAL_ID_SCHEME)
            .map_err(unknown)?
            .into_iter()
            .zip(report.findings())
            .map(|(id, finding)| (id.uuid(), (id, finding)))
            .collect();
    let outcomes = not_evaluated_ids(report, project, IFC_GLOBAL_ID_SCHEME).map_err(unknown)?;
    let mut decisions = Vec::new();
    let mut unmatched = Vec::new();
    for markup in topics {
        let topic = &markup.topic;
        let skip = |reason| UnmatchedTopic {
            guid: topic.guid.clone(),
            title: topic.title.clone(),
            status: topic.topic_status.clone(),
            reason,
        };
        let Some(guid) = topic.guid.as_deref().and_then(|g| Uuid::try_parse(g).ok()) else {
            unmatched.push(skip(Unmatched::NoGuid));
            continue;
        };
        let Some(finding) = findings.get(&guid) else {
            let reason = if outcomes.contains(&guid) {
                Unmatched::NotEvaluated
            } else {
                Unmatched::NoFinding
            };
            unmatched.push(skip(reason));
            continue;
        };
        match decision(markup, finding.0, finding.1, rule_labels) {
            Ok(Some(decision)) => decisions.push(decision),
            Ok(None) => {}
            Err(why) => unmatched.push(skip(Unmatched::Unreadable(why))),
        }
    }
    Ok(Import {
        decisions: Decisions::new(decisions)?,
        unmatched,
    })
}

fn unknown(error: IdentityError) -> ImportError {
    match error {
        IdentityError::UnknownObject(object) => ImportError::UnknownObject(object),
        // Identities are derived here, never parsed.
        IdentityError::NotAnIdentity(text) => unreachable!("parsed no identity: {text}"),
    }
}

/// GUID of the comment the export writes for a finding's decision, derived
/// from the topic's so re-exporting reproduces it.
pub(crate) fn decision_comment_guid(topic: Uuid) -> Uuid {
    Uuid::new_v5(&topic, b"decision")
}

/// GUIDs of the thread comments the export writes without an identity of
/// their own: UUIDv5 in the topic's namespace over author, date, text and
/// how often that same comment came before, so re-exporting reproduces them
/// and importing recognises them.
#[derive(Default)]
pub(crate) struct ThreadGuids {
    seen: BTreeMap<String, usize>,
}

impl ThreadGuids {
    pub(crate) fn next(&mut self, topic: Uuid, author: &str, date: DateTime, text: &str) -> Uuid {
        let key = format!("comment\n{author}\n{date}\n{text}");
        let count = self.seen.entry(key.clone()).or_default();
        let guid = Uuid::new_v5(&topic, format!("{key}\n{count}").as_bytes());
        *count += 1;
        guid
    }
}

/// The decision status a topic status stands for.
fn status(topic_status: Option<&str>) -> DecisionStatus {
    let is = |set: &[&str]| {
        topic_status.is_some_and(|status| {
            set.iter()
                .any(|known| known.eq_ignore_ascii_case(status.trim()))
        })
    };
    if is(&ACCEPTED_STATUSES) {
        DecisionStatus::Accepted
    } else if is(&REJECTED_STATUSES) {
        DecisionStatus::Rejected
    } else {
        DecisionStatus::Open
    }
}

/// A who and when, read from a topic or comment.
fn stamp(
    author: Option<&str>,
    date: Option<&str>,
    what: &str,
) -> Result<(String, DateTime), String> {
    let author = author
        .map(str::trim)
        .filter(|author| !author.is_empty())
        .ok_or_else(|| format!("{what} has no author"))?;
    let date = date.ok_or_else(|| format!("{what} has no date"))?;
    let date = date
        .trim()
        .parse()
        .map_err(|error| format!("{what}: {error}"))?;
    Ok((author.to_owned(), date))
}

/// The reviewer's text of the export's decision comment: without the status
/// word it starts with and the change note it may end with.
fn decision_text(comment: &str) -> &str {
    let text = comment
        .split_once("\nChanged since the decision: ")
        .map_or(comment, |(text, _)| text);
    for word in ["Open", "Accepted", "Rejected"] {
        if text == word {
            return "";
        }
        if let Some(rest) = text
            .strip_prefix(word)
            .and_then(|rest| rest.strip_prefix(": "))
        {
            return rest.trim();
        }
    }
    text.trim()
}

/// What a topic sets beside status and comments.
struct Review {
    assigned_to: Option<String>,
    due_date: Option<DateTime>,
    priority: Option<String>,
    labels: Vec<String>,
}

impl Review {
    fn read(
        topic: &openbim_bcf::Topic,
        found: &Finding,
        rule_labels: &BTreeMap<String, Vec<String>>,
    ) -> Result<Self, String> {
        let text = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        let due_date = text(topic.due_date.as_deref())
            .map(|date| {
                date.parse()
                    .map_err(|error| format!("the due date: {error}"))
            })
            .transpose()?;
        let exported = exported_labels(found, rule_labels);
        let mut labels: Vec<String> = Vec::new();
        for label in topic.labels.iter().map(|label| label.trim()) {
            let exporters = label.is_empty()
                || exported.iter().any(|known| known == label)
                || EXPORTED_LABEL_PREFIXES
                    .iter()
                    .any(|prefix| label.starts_with(prefix))
                || labels.iter().any(|known| known == label);
            if !exporters {
                labels.push(label.to_owned());
            }
        }
        Ok(Self {
            assigned_to: text(topic.assigned_to.as_deref()),
            due_date,
            priority: text(topic.priority.as_deref())
                .filter(|given| given != priority(&found.severity)),
            labels,
        })
    }

    fn is_empty(&self) -> bool {
        self.assigned_to.is_none()
            && self.due_date.is_none()
            && self.priority.is_none()
            && self.labels.is_empty()
    }
}

/// A reviewer's comment: author, date, trimmed text and GUID as written.
type Said<'a> = (String, DateTime, &'a str, Option<&'a str>);

/// Who decided when: the latest of the export's decision comment and the
/// topic's modification; without either, the last comment; without
/// comments, the topic's creation.
fn decided_by(
    topic: &openbim_bcf::Topic,
    own: Option<&Comment>,
    comments: &[Said<'_>],
) -> Result<(String, DateTime), String> {
    let mut candidates = Vec::new();
    if let Some(own) = own {
        candidates.push(stamp(
            own.author.as_deref(),
            own.date.as_deref(),
            "the decision comment",
        )?);
    }
    if topic.modified_author.is_some() || topic.modified_date.is_some() {
        candidates.push(stamp(
            topic.modified_author.as_deref(),
            topic.modified_date.as_deref(),
            "the topic's modification",
        )?);
    }
    if candidates.is_empty() {
        return match comments.last() {
            Some((author, date, _, _)) => Ok((author.clone(), *date)),
            None => stamp(
                topic.creation_author.as_deref(),
                topic.creation_date.as_deref(),
                "the topic",
            ),
        };
    }
    Ok(candidates
        .into_iter()
        .reduce(|latest, next| {
            if next.1.cmp_instant(latest.1).is_gt() {
                next
            } else {
                latest
            }
        })
        .expect("a candidate was pushed"))
}

/// The decision a topic about `finding` carries; `None` when it carries no
/// review state.
fn decision(
    markup: &Markup,
    finding: FindingId,
    found: &Finding,
    rule_labels: &BTreeMap<String, Vec<String>>,
) -> Result<Option<Decision>, String> {
    let own_guid = decision_comment_guid(finding.uuid());
    let is_own = |comment: &Comment| {
        comment
            .guid
            .as_deref()
            .and_then(|guid| Uuid::try_parse(guid).ok())
            == Some(own_guid)
    };
    let own = markup.comments.iter().find(|comment| is_own(comment));
    let mut comments: Vec<Said<'_>> = Vec::new();
    for comment in markup.comments.iter().filter(|comment| !is_own(comment)) {
        let (author, date) = stamp(
            comment.author.as_deref(),
            comment.date.as_deref(),
            "a comment",
        )?;
        let text = comment
            .comment
            .as_deref()
            .map(str::trim)
            .unwrap_or_default();
        comments.push((author, date, text, comment.guid.as_deref()));
    }
    let topic = &markup.topic;
    let status = status(topic.topic_status.as_deref());
    let review = Review::read(topic, found, rule_labels)?;
    if status == DecisionStatus::Open && own.is_none() && comments.is_empty() && review.is_empty() {
        return Ok(None);
    }
    let (author, date) = decided_by(topic, own, &comments)?;
    let mut thread = Vec::new();
    if let Some(own) = own {
        let text = own
            .comment
            .as_deref()
            .map(decision_text)
            .unwrap_or_default();
        if !text.is_empty() {
            let (author, date) = stamp(
                own.author.as_deref(),
                own.date.as_deref(),
                "the decision comment",
            )?;
            thread.push(DecisionComment::new(author, date, text));
        }
    }
    let mut guids = ThreadGuids::default();
    for (author, date, text, guid) in comments {
        if text.is_empty() {
            continue;
        }
        let derived = guids.next(finding.uuid(), &author, date, text);
        let id = guid
            .and_then(|guid| Uuid::try_parse(guid).ok())
            .filter(|guid| *guid != derived);
        thread.push(DecisionComment {
            author,
            date,
            text: text.to_owned(),
            id,
        });
    }
    let mut decision = Decision::new(finding, status, author, date)
        .map_err(|error| error.to_string())?
        .with_labels(review.labels);
    decision.comments = thread;
    decision.assigned_to = review.assigned_to;
    decision.due_date = review.due_date;
    decision.priority = review.priority;
    Ok(Some(decision))
}

#[cfg(test)]
mod tests {
    use super::decision_text;

    #[test]
    fn the_decision_comment_text_loses_its_status_and_change_note() {
        assert_eq!(decision_text("Accepted"), "");
        assert_eq!(decision_text("Rejected: a lining"), "a lining");
        assert_eq!(
            decision_text(
                "Accepted: agreed\nChanged since the decision: severity error -> warning"
            ),
            "agreed"
        );
        assert_eq!(decision_text("edited elsewhere"), "edited elsewhere");
    }
}
