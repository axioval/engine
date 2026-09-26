#![allow(clippy::doc_markdown)]

//! BCF issue archives from Axioval reports.
//!
//! An output sink, not a source adapter: it reads a finished [`Report`] and
//! the [`Project`] it was computed over, and depends on nothing else in the
//! engine. Each finding becomes one BCF 2.1 topic. Each not-evaluated outcome
//! becomes one too, because an archive that lists only findings reads as
//! "everything else passed" when it did not.
//!
//! # Identity
//!
//! BCF names components by IFC GlobalId, which the IFC adapter attaches as an
//! [`ExternalId`](axioval_ir::ExternalId) in the [`IFC_GLOBAL_ID_SCHEME`]. An
//! object without that alias cannot be selected in a viewpoint. Its topic is
//! still written, naming the object by its source-qualified id in the
//! description, and the object is listed in [`Export::unanchored`] so the
//! host can say so rather than let the gap pass unnoticed.
//!
//! # Determinism
//!
//! The caller supplies the author and timestamp; nothing reads the clock.
//! Topic and viewpoint GUIDs are UUIDv5 over the rule, the objects' GlobalIds
//! (source-qualified ids where there is none), and the message. Because a
//! GlobalId survives re-export, rechecking a revised model reproduces the
//! GUIDs of issues that are still there, and a BCF tool can track them.
//!
//! # Version
//!
//! Only BCF 2.1 is written. BCF 3.0 requires a camera on every viewpoint, and
//! a report carries no geometry to place one.

use std::collections::{BTreeMap, BTreeSet};

use axioval_ir::{Finding, NotEvaluated, NotEvaluatedReason, ObjectId, Project, Report, Severity};
use openbim_bcf::Component;
use openbim_bcf::write::{self, Document, TargetVersion, Topic, Viewpoint, WriteError};
use thiserror::Error;
use uuid::Uuid;

/// External id scheme whose values are IFC GlobalIds.
///
/// Must equal the IFC adapter's `IFC_GLOBAL_ID`; a test pins the two together
/// so this crate need not depend on the adapter.
pub const IFC_GLOBAL_ID_SCHEME: &str = "ifc-globalid";

/// `TopicType` of a topic made from a not-evaluated outcome.
pub const NOT_EVALUATED_TOPIC_TYPE: &str = "Not evaluated";

/// Namespace of every GUID this crate derives. Changing it changes every GUID.
const NAMESPACE: Uuid = Uuid::from_u128(0x6b1f_5a0e_2c3d_4e8f_9a71_0d2c_5e4b_8f13);

/// What the caller decides about every topic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// `CreationAuthor` of every topic, e.g. the checking tool or its operator.
    pub author: String,
    /// `CreationDate` of every topic, an `xs:dateTime` such as
    /// `2026-09-26T10:00:00Z`.
    pub date: String,
    /// `TopicStatus` of every topic.
    pub status: String,
    /// Whether not-evaluated outcomes become topics.
    pub include_not_evaluated: bool,
}

impl Options {
    /// Open topics by `author` at `date`, not-evaluated outcomes included.
    pub fn new(author: impl Into<String>, date: impl Into<String>) -> Self {
        Self {
            author: author.into(),
            date: date.into(),
            status: "Open".to_owned(),
            include_not_evaluated: true,
        }
    }
}

/// Why a report could not be exported.
#[derive(Debug, Error)]
pub enum ExportError {
    /// The report names an object the project does not contain, so the
    /// report was not computed over this project.
    #[error("report names {0}, which is not in the project")]
    UnknownObject(ObjectId),
    /// The BCF writer refused the document; nothing was written.
    #[error("BCF writer refused the document: {0}")]
    Write(#[from] WriteError),
}

/// A report mapped onto BCF, ready to write.
#[derive(Debug)]
pub struct Export {
    /// The BCF document, one topic per exported report entry, in report order.
    pub document: Document,
    /// Objects named by the report that no viewpoint could select, because
    /// they carry no GlobalId alias. Sorted and deduplicated.
    pub unanchored: Vec<ObjectId>,
}

impl Export {
    /// Writes the document as `.bcfzip` bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError::Write`] when the writer refuses the document,
    /// e.g. a date that is not an `xs:dateTime` or two identical entries.
    pub fn to_bytes(&self) -> Result<Vec<u8>, ExportError> {
        Ok(write::to_vec(&self.document)?)
    }
}

/// Maps `report`, computed over `project`, onto a BCF 2.1 document.
///
/// # Errors
///
/// Returns [`ExportError::UnknownObject`] when the report names an object the
/// project does not contain.
pub fn export(
    report: &Report,
    project: &Project,
    options: &Options,
) -> Result<Export, ExportError> {
    let mut entries = Vec::new();
    for finding in report.findings() {
        entries.push(Entry::finding(finding, project)?);
    }
    if options.include_not_evaluated {
        for outcome in report.not_evaluated() {
            entries.push(Entry::not_evaluated(outcome, project)?);
        }
    }

    // A key made only of GlobalIds can repeat across sources, e.g. two
    // revisions of one model federated in one project. Those keys, and only
    // those, are qualified by source so every GUID stays unique.
    let mut uses: BTreeMap<&str, usize> = BTreeMap::new();
    for entry in &entries {
        *uses.entry(entry.key.as_str()).or_default() += 1;
    }
    let qualified: Vec<bool> = entries
        .iter()
        .map(|entry| uses[entry.key.as_str()] > 1)
        .collect();

    let mut unanchored = BTreeSet::new();
    let topics = entries
        .iter()
        .zip(qualified)
        .map(|(entry, qualified)| {
            let key = if qualified {
                format!("{}\n{}", entry.key, entry.sources)
            } else {
                entry.key.clone()
            };
            unanchored.extend(entry.unanchored.iter().cloned());
            entry.topic(&key, options)
        })
        .collect();
    Ok(Export {
        document: Document {
            version: TargetVersion::V2_1,
            extensions: None,
            topics,
        },
        unanchored: unanchored.into_iter().collect(),
    })
}

/// One report entry, resolved against the project.
struct Entry {
    title: String,
    topic_type: String,
    label: String,
    description: String,
    /// GUID input without source qualification.
    key: String,
    /// Every source the entry touches, for disambiguating a repeated key.
    sources: String,
    selection: Vec<Component>,
    unanchored: Vec<ObjectId>,
}

impl Entry {
    fn finding(finding: &Finding, project: &Project) -> Result<Self, ExportError> {
        let mut objects = vec![&finding.object_id];
        objects.extend(&finding.related);
        let resolved = Resolved::new(&objects, project)?;
        let mut description = vec![
            finding.message.trim().to_owned(),
            format!("Rule: {}", finding.rule_id),
            format!("Object: {}", finding.object_id),
        ];
        if !finding.related.is_empty() {
            description.push(format!("Related: {}", join(&finding.related)));
        }
        for evidence in &finding.evidence {
            let exactness = if evidence.exact { "exact" } else { "inexact" };
            description.push(format!("Evidence ({exactness}): {}", evidence.locator));
        }
        Ok(Self {
            title: title(&finding.message, &finding.rule_id.to_string()),
            topic_type: severity(&finding.severity).to_owned(),
            label: finding.rule_id.to_string(),
            description: description.join("\n"),
            key: format!(
                "finding\n{}\n{}\n{}",
                finding.rule_id, resolved.key, finding.message
            ),
            sources: resolved.sources,
            selection: resolved.selection,
            unanchored: resolved.unanchored,
        })
    }

    fn not_evaluated(outcome: &NotEvaluated, project: &Project) -> Result<Self, ExportError> {
        let objects: Vec<&ObjectId> = outcome.object_id.iter().collect();
        let resolved = Resolved::new(&objects, project)?;
        let reason = reason(&outcome.reason);
        let mut description = vec![
            outcome.message.trim().to_owned(),
            format!("Rule: {}", outcome.rule_id),
            format!("Reason: {reason}"),
        ];
        match &outcome.object_id {
            Some(object) => description.push(format!("Object: {object}")),
            None => description.push("Object: none; the whole rule was not evaluated".to_owned()),
        }
        Ok(Self {
            title: title(&outcome.message, &outcome.rule_id.to_string()),
            topic_type: NOT_EVALUATED_TOPIC_TYPE.to_owned(),
            label: outcome.rule_id.to_string(),
            description: description.join("\n"),
            key: format!(
                "not-evaluated\n{}\n{}\n{reason}\n{}",
                outcome.rule_id, resolved.key, outcome.message
            ),
            sources: resolved.sources,
            selection: resolved.selection,
            unanchored: resolved.unanchored,
        })
    }

    fn topic(&self, key: &str, options: &Options) -> Topic {
        let guid = Uuid::new_v5(&NAMESPACE, key.as_bytes());
        let viewpoints = if self.selection.is_empty() {
            vec![]
        } else {
            vec![Viewpoint {
                guid: Uuid::new_v5(&guid, b"viewpoint").to_string(),
                selection: self.selection.clone(),
                camera: None,
            }]
        };
        Topic {
            guid: guid.to_string(),
            title: self.title.clone(),
            description: Some(self.description.clone()),
            topic_type: Some(self.topic_type.clone()),
            topic_status: Some(options.status.clone()),
            labels: vec![self.label.clone()],
            creation_date: options.date.clone(),
            creation_author: options.author.clone(),
            viewpoints,
            ..Topic::default()
        }
    }
}

/// The objects of one entry, subject first, mapped to BCF components.
struct Resolved {
    key: String,
    sources: String,
    selection: Vec<Component>,
    unanchored: Vec<ObjectId>,
}

impl Resolved {
    fn new(objects: &[&ObjectId], project: &Project) -> Result<Self, ExportError> {
        let mut keys = Vec::new();
        let mut sources = BTreeSet::new();
        let mut selection = Vec::new();
        let mut unanchored = Vec::new();
        for (index, id) in objects.iter().enumerate() {
            let object = project
                .object(id)
                .ok_or_else(|| ExportError::UnknownObject((*id).clone()))?;
            sources.insert(id.source.to_string());
            if let Some(global_id) = object.external_id(IFC_GLOBAL_ID_SCHEME) {
                keys.push(global_id.to_owned());
                // A viewpoint of only the related objects would show the
                // reviewer the slab, not the wall that fails to rest on it.
                if index == 0 || !selection.is_empty() {
                    selection.push(Component::ifc(global_id));
                }
            } else {
                keys.push(id.to_string());
                unanchored.push((*id).clone());
            }
        }
        Ok(Self {
            key: keys.join("\n"),
            sources: sources.into_iter().collect::<Vec<_>>().join("\n"),
            selection,
            unanchored,
        })
    }
}

/// A title the writer accepts: the message, or the rule id when it is blank.
fn title(message: &str, rule: &str) -> String {
    let message = message.trim();
    if message.is_empty() {
        rule.to_owned()
    } else {
        message.to_owned()
    }
}

fn severity(severity: &Severity) -> &'static str {
    match severity {
        Severity::Error => "Error",
        Severity::Warning => "Warning",
        Severity::Info => "Info",
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
    }
}

fn join(ids: &[ObjectId]) -> String {
    ids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
