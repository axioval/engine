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
//! A resource object the report names ([`Report::resources`]: a material, a
//! classification, a relationship) is never a component, since no viewer
//! shows it as an element. Its topic is written the same way, without a
//! viewpoint, and it is listed in [`Export::unanchored`].
//!
//! # Model-level topics
//!
//! A finding or outcome scoped to a source or the project (see
//! [`Scope`]) names no subject, so its topic has no
//! viewpoint and no component: selecting only the related objects would
//! point the reviewer at the wrong thing. The description names the scope and
//! lists the related objects instead.
//!
//! # Determinism
//!
//! The caller supplies the author and timestamp; nothing reads the clock.
//! Topic and viewpoint GUIDs are UUIDv5 over the rule, the objects' GlobalIds
//! (source-qualified ids where there is none), and the message. Because a
//! GlobalId survives re-export, rechecking a revised model reproduces the
//! GUIDs of issues that are still there, and a BCF tool can track them. A
//! finding's topic GUID is its [`FindingId`](axioval_ir::FindingId) over
//! [`IFC_GLOBAL_ID_SCHEME`], the identity a host records decisions against.
//!
//! # Decisions
//!
//! A finding carrying a reviewer's decision writes it into its topic: status
//! [`STATUS_ACCEPTED`] or [`STATUS_REJECTED`] (an open decision keeps
//! [`Options::status`]), one comment by the decision's author at its date,
//! and [`DECISION_CHANGED_LABEL`] when the finding changed since.
//!
//! # Import
//!
//! [`import`] reads an archive back, typically one written here and then
//! reviewed in another BCF tool, and maps each topic whose GUID is a
//! finding's identity onto a [`Decision`](axioval_ir::Decision): a closed or
//! accepted topic accepts the finding, a rejected one rejects it, and
//! comments are carried. Topics that decide no current finding are listed in
//! [`Import::unmatched`], never dropped.
//!
//! # Cameras
//!
//! A report carries no geometry. A host that measured the model passes each
//! object's [`Bounds`] in [`Options::bounds`]; a topic whose objects are all
//! bounded then gets two viewpoints, a perspective and an orthogonal camera
//! fitted to the union of those bounds (see [`Bounds`] for the fit). A
//! missing bound leaves the viewpoint without a camera rather than a guessed
//! one, and the object is listed in [`Export::unframed`].
//!
//! # Colouring
//!
//! With [`Options::colors`], every viewpoint colours the finding's subject in
//! [`Colors::subject`] and its related objects in [`Colors::related`], so a
//! reviewer tells the element that fails from the ones it fails against.
//! The defaults are [`SUBJECT_COLOR`] and [`RELATED_COLOR`]. `None`, the
//! default, writes no colouring, exactly as before.
//!
//! # Visibility
//!
//! With [`Options::isolate`], every viewpoint hides the whole model except
//! the objects it selects: `DefaultVisibility` false, with the subject and
//! related objects as exceptions. Off by default, which shows everything,
//! as before.
//!
//! # Section box
//!
//! With [`Options::section_box`], a viewpoint with a fitted camera is also
//! cut by six clipping planes: a box around the union of its objects'
//! bounds, grown by [`SECTION_BOX_MARGIN_METRES`] on every side, so walls
//! and slabs around the finding no longer hide it. A viewpoint without a
//! camera has no bounds to box and is never clipped.
//!
//! # Version
//!
//! BCF 2.1 by default. BCF 3.0 ([`Version::V3_0`]) requires a camera on every
//! viewpoint, so it is written only when every viewpoint has one; otherwise
//! the export is refused with [`ExportError::MissingCamera`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use axioval_ir::contract::{RuleFolder, RuleSetPackage};
use axioval_ir::{
    DecisionStatus, EvidenceCheck, Finding, FindingDecision, IdentityError, Location, NotEvaluated,
    NotEvaluatedReason, ObjectId, Place, Project, Report, Scope, Severity, finding_ids,
    not_evaluated_ids,
};
use openbim_bcf::Component;
use openbim_bcf::write::{
    self, Camera, ClippingPlane, Coloring, Comment, Document, Projection, TargetVersion, Topic,
    Vector3, Viewpoint, Visibility, WriteError,
};
use thiserror::Error;
use uuid::Uuid;

mod import;
use import::decision_comment_guid;
pub use import::{
    ACCEPTED_STATUSES, IMPORT_LIMITS, Import, ImportError, MAX_ATTRIBUTES_PER_TAG,
    REJECTED_STATUSES, Unmatched, UnmatchedTopic, import, import_topics,
};

/// External id scheme whose values are IFC GlobalIds.
///
/// Must equal the IFC adapter's `IFC_GLOBAL_ID`; the facade's `ifc_bcf` test
/// pins the two together so this crate need not depend on the adapter.
pub const IFC_GLOBAL_ID_SCHEME: &str = "ifc-globalid";

/// `TopicType` of a topic made from a not-evaluated outcome.
pub const NOT_EVALUATED_TOPIC_TYPE: &str = "Not evaluated";

/// `Priority` of an error finding's topic.
pub const PRIORITY_HIGH: &str = "High";
/// `Priority` of a warning finding's topic.
pub const PRIORITY_NORMAL: &str = "Normal";
/// `Priority` of an info finding's topic.
pub const PRIORITY_LOW: &str = "Low";

/// `TopicStatus` of a finding's topic when the finding was accepted.
pub const STATUS_ACCEPTED: &str = "Accepted";
/// `TopicStatus` of a finding's topic when the finding was rejected.
pub const STATUS_REJECTED: &str = "Rejected";
/// Label of a decided finding's topic whose evidence changed since the
/// decision, so a reviewer can filter what to look at again.
pub const DECISION_CHANGED_LABEL: &str = "Decision changed";

/// Vertical field of view of every perspective camera, in degrees: the
/// widest BCF 2.1 allows, and valid in 3.0.
pub const FIELD_OF_VIEW_DEGREES: f64 = 60.0;

/// Width over height of every BCF 3.0 camera. 2.1 has no aspect ratio.
///
/// Square, so the fit holds in both directions; a wider view only shows
/// more around the objects.
pub const ASPECT_RATIO: f64 = 1.0;

/// How much room a fitted camera leaves around the objects: the bounding
/// sphere's radius is scaled by this before it is framed.
pub const FRAME_MARGIN: f64 = 1.2;

/// Smallest radius a camera frames, in metres, so a point-like object is
/// shown with some of its surroundings instead of filling the view.
pub const MIN_FRAME_RADIUS_METRES: f64 = 0.5;

/// Default colour of a finding's subject: opaque red.
pub const SUBJECT_COLOR: Color = Color::argb(0xFFFF_0000);

/// Default colour of the objects a finding relates its subject to: opaque
/// blue.
pub const RELATED_COLOR: Color = Color::argb(0xFF00_00FF);

/// How far a section box reaches beyond the objects' bounds on every side,
/// in metres, so the objects are shown with their immediate surroundings
/// and never cut themselves.
pub const SECTION_BOX_MARGIN_METRES: f64 = 0.5;

/// Coordinates of fitted cameras are rounded to micrometres, so written
/// numbers stay short and stable.
const DECIMALS: f64 = 1e6;

/// The BCF version written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Version {
    /// BCF 2.1; a camera is optional.
    #[default]
    V2_1,
    /// BCF 3.0; every viewpoint needs a camera, so every object of a
    /// selected topic needs bounds.
    V3_0,
}

impl From<Version> for TargetVersion {
    fn from(version: Version) -> Self {
        match version {
            Version::V2_1 => Self::V2_1,
            Version::V3_0 => Self::V3_0,
        }
    }
}

/// A colour as alpha, red, green and blue, written as 8 uppercase hex
/// digits (`AARRGGBB`), the form both BCF 2.1 and 3.0 accept.
///
/// Parsed from 6 (`RRGGBB`, opaque) or 8 (`AARRGGBB`) hex digits in either
/// case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color(u32);

impl Color {
    /// The colour of `value`, `0xAARRGGBB`.
    #[must_use]
    pub const fn argb(value: u32) -> Self {
        Self(value)
    }
    /// The colour as `0xAARRGGBB`.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:08X}", self.0)
    }
}

impl FromStr for Color {
    type Err = ParseColorError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || ParseColorError(text.to_owned());
        if !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        let value = u32::from_str_radix(text, 16).map_err(|_| invalid())?;
        match text.len() {
            6 => Ok(Self(0xFF00_0000 | value)),
            8 => Ok(Self(value)),
            _ => Err(invalid()),
        }
    }
}

/// A colour that is not 6 or 8 hex digits.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("`{0}` is not a colour: expected 6 (RRGGBB) or 8 (AARRGGBB) hex digits")]
pub struct ParseColorError(String);

/// The colours of a viewpoint's objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Colors {
    /// The finding's subject, or the object a not-evaluated outcome names.
    pub subject: Color,
    /// The objects the finding relates its subject to.
    pub related: Color,
}

impl Default for Colors {
    /// [`SUBJECT_COLOR`] and [`RELATED_COLOR`].
    fn default() -> Self {
        Self {
            subject: SUBJECT_COLOR,
            related: RELATED_COLOR,
        }
    }
}

/// The axis-aligned extent of one object in model coordinates, in metres,
/// as the host measured it.
///
/// A topic's cameras frame the union of its objects' bounds: its centre is
/// looked at from above, south and east (direction `(-1, 1, -1)`, up
/// `(-1, 1, 2)`), from where a sphere around the union, its radius scaled by
/// [`FRAME_MARGIN`] and at least [`MIN_FRAME_RADIUS_METRES`], just fits the
/// [`FIELD_OF_VIEW_DEGREES`]. The orthogonal camera stands at the same point
/// and shows the sphere's diameter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    min: [f64; 3],
    max: [f64; 3],
}

impl Bounds {
    /// Bounds from their corners, `None` when a coordinate is not finite or
    /// `min` exceeds `max` on an axis.
    #[must_use]
    pub fn new(min: [f64; 3], max: [f64; 3]) -> Option<Self> {
        (0..3)
            .all(|axis| min[axis].is_finite() && max[axis].is_finite() && min[axis] <= max[axis])
            .then_some(Self { min, max })
    }
    /// The lowest corner.
    #[must_use]
    pub fn min(&self) -> [f64; 3] {
        self.min
    }
    /// The highest corner.
    #[must_use]
    pub fn max(&self) -> [f64; 3] {
        self.max
    }
    fn union(&self, other: &Self) -> Self {
        Self {
            min: [0, 1, 2].map(|axis| self.min[axis].min(other.min[axis])),
            max: [0, 1, 2].map(|axis| self.max[axis].max(other.max[axis])),
        }
    }
}

/// What the caller decides about every topic.
#[derive(Clone, Debug, PartialEq)]
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
    /// The BCF version written.
    pub version: Version,
    /// Each object's measured bounds, when the host measured the model.
    ///
    /// `None` writes viewpoints without cameras, exactly as before bounds
    /// existed. `Some` fits cameras to every viewpoint whose objects are all
    /// bounded; an object missing from the map leaves its viewpoint without
    /// a camera and is listed in [`Export::unframed`].
    pub bounds: Option<BTreeMap<ObjectId, Bounds>>,
    /// Labels each rule's topics carry after the rule id, keyed by the rule
    /// id as the report names it: its ruleset folder path and tags, see
    /// [`ruleset_labels`]. Empty adds none.
    pub rule_labels: BTreeMap<String, Vec<String>>,
    /// Colours of the subject and the related objects in every viewpoint.
    /// `None` writes no colouring, exactly as before colouring existed.
    pub colors: Option<Colors>,
    /// Whether every viewpoint shows only the objects it selects, the
    /// subject and related objects, hiding the rest of the model. `false`
    /// shows everything, exactly as before visibility existed.
    pub isolate: bool,
    /// Whether every viewpoint with a fitted camera is cut by a section box
    /// around its objects' bounds (see [`SECTION_BOX_MARGIN_METRES`]).
    /// Without bounds no viewpoint is clipped. `false` writes no clipping
    /// planes, exactly as before.
    pub section_box: bool,
}

impl Options {
    /// Open topics by `author` at `date`, not-evaluated outcomes included.
    pub fn new(author: impl Into<String>, date: impl Into<String>) -> Self {
        Self {
            author: author.into(),
            date: date.into(),
            status: "Open".to_owned(),
            include_not_evaluated: true,
            version: Version::V2_1,
            bounds: None,
            rule_labels: BTreeMap::new(),
            colors: None,
            isolate: false,
            section_box: false,
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
    /// BCF 3.0 was asked for, but a viewpoint has no camera because one of
    /// its objects has no bounds. Nothing was written.
    #[error("BCF 3.0 needs a camera on every viewpoint, but {object} has no bounds to fit one to")]
    MissingCamera {
        /// An object of the viewpoint without bounds.
        object: ObjectId,
    },
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
    /// they carry no GlobalId alias or are resource objects. Sorted and
    /// deduplicated.
    pub unanchored: Vec<ObjectId>,
    /// Objects whose viewpoint got no camera because [`Options::bounds`]
    /// has none for them. Empty when no bounds were supplied at all. Sorted
    /// and deduplicated.
    pub unframed: Vec<ObjectId>,
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

/// Maps `report`, computed over `project`, onto a BCF document of
/// [`Options::version`].
///
/// # Errors
///
/// Returns [`ExportError::UnknownObject`] when the report names an object the
/// project does not contain, and [`ExportError::MissingCamera`] when BCF 3.0
/// is asked for and a viewpoint has no camera.
pub fn export(
    report: &Report,
    project: &Project,
    options: &Options,
) -> Result<Export, ExportError> {
    // Topic GUIDs are the outcomes' stable identities over GlobalIds: the
    // identities a host records decisions against.
    let mut entries = Vec::new();
    let ids = finding_ids(report, project, IFC_GLOBAL_ID_SCHEME).map_err(unknown)?;
    for (finding, id) in report.findings().iter().zip(ids) {
        entries.push((Entry::finding(finding, (report, project))?, id.uuid()));
    }
    if options.include_not_evaluated {
        let ids = not_evaluated_ids(report, project, IFC_GLOBAL_ID_SCHEME).map_err(unknown)?;
        for (outcome, id) in report.not_evaluated().iter().zip(ids) {
            entries.push((Entry::not_evaluated(outcome, (report, project))?, id));
        }
    }

    let mut unanchored = BTreeSet::new();
    let mut unframed = BTreeSet::new();
    let mut topics = Vec::with_capacity(entries.len());
    for (entry, guid) in &entries {
        unanchored.extend(entry.unanchored.iter().cloned());
        let (topic, uncamered) = entry.topic(*guid, options);
        match uncamered {
            Uncamered::No => {}
            Uncamered::NoBounds if options.version == Version::V2_1 => {}
            Uncamered::Unbounded(object) if options.version == Version::V2_1 => {
                unframed.insert(object);
            }
            Uncamered::NoBounds => {
                let object = entry.framed[0].clone();
                return Err(ExportError::MissingCamera { object });
            }
            Uncamered::Unbounded(object) => return Err(ExportError::MissingCamera { object }),
        }
        topics.push(topic);
    }
    Ok(Export {
        document: Document {
            version: options.version.into(),
            extensions: None,
            topics,
        },
        unanchored: unanchored.into_iter().collect(),
        unframed: unframed.into_iter().collect(),
    })
}

fn unknown(error: IdentityError) -> ExportError {
    match error {
        IdentityError::UnknownObject(object) => ExportError::UnknownObject(object),
        // Identities are derived here, never parsed.
        IdentityError::NotAnIdentity(text) => unreachable!("parsed no identity: {text}"),
    }
}

/// One report entry, resolved against the project.
struct Entry {
    title: String,
    topic_type: String,
    priority: Option<&'static str>,
    /// The rule id, then the location's storeys and spaces.
    labels: Vec<String>,
    description: String,
    /// The reviewer's decision, for a decided finding.
    decision: Option<FindingDecision>,
    selection: Vec<Component>,
    /// Every object a camera frames: the subject and related objects.
    framed: Vec<ObjectId>,
    unanchored: Vec<ObjectId>,
}

/// Why a topic's viewpoint has no camera, if it has one without.
enum Uncamered {
    /// Every viewpoint has a camera, or the topic has none.
    No,
    /// The host supplied no bounds at all.
    NoBounds,
    /// The host supplied bounds, but none for this object.
    Unbounded(ObjectId),
}

impl Entry {
    fn finding(finding: &Finding, known: (&Report, &Project)) -> Result<Self, ExportError> {
        let subject = finding.object_id();
        let mut objects: Vec<&ObjectId> = subject.into_iter().collect();
        objects.extend(&finding.related);
        let resolved = Resolved::new(&objects, subject.is_some(), known)?;
        let mut description = vec![
            finding.message.trim().to_owned(),
            format!("Rule: {}", finding.rule_id),
            match &finding.scope {
                Scope::Object(object) => format!("Object: {object}"),
                Scope::Source(source) => format!("Source: {source}; no single object"),
                Scope::Project => "Project: no single object or source".to_owned(),
            },
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
            priority: Some(priority(&finding.severity)),
            labels: {
                let mut labels = labels(&finding.rule_id.to_string(), finding.location.as_ref());
                if !finding.categories.is_empty() {
                    let path = finding.categories.join(" / ");
                    labels.insert(1, format!("Category: {path}"));
                }
                labels
            },
            description: description.join("\n"),
            decision: finding.decision.clone(),
            selection: resolved.selection,
            framed: objects.into_iter().cloned().collect(),
            unanchored: resolved.unanchored,
        })
    }

    fn not_evaluated(
        outcome: &NotEvaluated,
        known: (&Report, &Project),
    ) -> Result<Self, ExportError> {
        let objects: Vec<&ObjectId> = outcome.object_id().into_iter().collect();
        let resolved = Resolved::new(&objects, true, known)?;
        let reason = reason(&outcome.reason);
        let mut description = vec![
            outcome.message.trim().to_owned(),
            format!("Rule: {}", outcome.rule_id),
            format!("Reason: {reason}"),
        ];
        match &outcome.scope {
            Scope::Object(object) => description.push(format!("Object: {object}")),
            Scope::Source(source) => description.push(format!(
                "Source: {source}; the rule was not evaluated for this source"
            )),
            Scope::Project => {
                description.push("Object: none; the whole rule was not evaluated".to_owned());
            }
        }
        Ok(Self {
            title: title(&outcome.message, &outcome.rule_id.to_string()),
            topic_type: NOT_EVALUATED_TOPIC_TYPE.to_owned(),
            // No severity was decided, so none is claimed.
            priority: None,
            labels: labels(&outcome.rule_id.to_string(), outcome.location.as_ref()),
            description: description.join("\n"),
            decision: None,
            selection: resolved.selection,
            framed: objects.into_iter().cloned().collect(),
            unanchored: resolved.unanchored,
        })
    }

    /// The topic, and whether a viewpoint of it has no camera.
    fn topic(&self, guid: Uuid, options: &Options) -> (Topic, Uncamered) {
        let mut uncamered = Uncamered::No;
        let viewpoints = if self.selection.is_empty() {
            vec![]
        } else {
            let viewpoint = |name: &[u8], camera, clipping_planes| Viewpoint {
                guid: Uuid::new_v5(&guid, name).to_string(),
                selection: self.selection.clone(),
                camera,
                coloring: options
                    .colors
                    .map(|colors| self.coloring(colors))
                    .unwrap_or_default(),
                visibility: options.isolate.then(|| Visibility {
                    default_visibility: false,
                    exceptions: self.selection.clone(),
                }),
                clipping_planes,
            };
            match self.frame(options.bounds.as_ref()) {
                Ok(frame) => {
                    let planes = if options.section_box {
                        frame.section_box()
                    } else {
                        Vec::new()
                    };
                    vec![
                        viewpoint(
                            b"viewpoint",
                            Some(frame.perspective(options.version)),
                            planes.clone(),
                        ),
                        viewpoint(
                            b"viewpoint-orthogonal",
                            Some(frame.orthogonal(options.version)),
                            planes,
                        ),
                    ]
                }
                Err(why) => {
                    uncamered = why;
                    vec![viewpoint(b"viewpoint", None, Vec::new())]
                }
            }
        };
        let mut labels = merged_labels(&self.labels, &options.rule_labels);
        let mut status = options.status.clone();
        let mut comments = Vec::new();
        if let Some(decision) = &self.decision {
            match decision.status {
                DecisionStatus::Open => {}
                DecisionStatus::Accepted => STATUS_ACCEPTED.clone_into(&mut status),
                DecisionStatus::Rejected => STATUS_REJECTED.clone_into(&mut status),
            }
            if decision.evidence == EvidenceCheck::Changed {
                labels.push(DECISION_CHANGED_LABEL.to_owned());
            }
            comments.push(decision_comment(decision, guid));
        }
        let topic = Topic {
            guid: guid.to_string(),
            title: self.title.clone(),
            description: Some(self.description.clone()),
            topic_type: Some(self.topic_type.clone()),
            topic_status: Some(status),
            priority: self.priority.map(str::to_owned),
            labels,
            creation_date: options.date.clone(),
            creation_author: options.author.clone(),
            comments,
            viewpoints,
        };
        (topic, uncamered)
    }

    /// The subject in its colour, then the related objects in theirs. The
    /// selection lists the subject first.
    fn coloring(&self, colors: Colors) -> Vec<Coloring> {
        let Some((subject, related)) = self.selection.split_first() else {
            return Vec::new();
        };
        let mut coloring = vec![Coloring {
            color: colors.subject.to_string(),
            components: vec![subject.clone()],
        }];
        if !related.is_empty() {
            coloring.push(Coloring {
                color: colors.related.to_string(),
                components: related.to_vec(),
            });
        }
        coloring
    }

    /// A sphere around the union of the framed objects' bounds.
    fn frame(&self, bounds: Option<&BTreeMap<ObjectId, Bounds>>) -> Result<Frame, Uncamered> {
        let bounds = bounds.ok_or(Uncamered::NoBounds)?;
        let mut union: Option<Bounds> = None;
        for object in &self.framed {
            let found = bounds
                .get(object)
                .ok_or_else(|| Uncamered::Unbounded(object.clone()))?;
            union = Some(union.map_or(*found, |union| union.union(found)));
        }
        // A selected topic always has its subject among the framed objects.
        union.map(Frame::new).ok_or(Uncamered::NoBounds)
    }
}

/// A sphere to fit cameras to.
struct Frame {
    /// The union of the framed objects' bounds.
    bounds: Bounds,
    centre: [f64; 3],
    radius: f64,
}

impl Frame {
    /// Looking down from above, south and east.
    const DIRECTION: [f64; 3] = [-1.0, 1.0, -1.0];
    /// World up, made perpendicular to [`Self::DIRECTION`].
    const UP: [f64; 3] = [-1.0, 1.0, 2.0];

    fn new(bounds: Bounds) -> Self {
        let centre = [0, 1, 2].map(|axis| f64::midpoint(bounds.min[axis], bounds.max[axis]));
        let diagonal = (0..3)
            .map(|axis| (bounds.max[axis] - bounds.min[axis]).powi(2))
            .sum::<f64>()
            .sqrt();
        Self {
            bounds,
            centre,
            radius: (diagonal / 2.0 * FRAME_MARGIN).max(MIN_FRAME_RADIUS_METRES),
        }
    }

    fn camera(&self, projection: Projection, version: Version) -> Camera {
        let unit = |v: [f64; 3]| {
            let length = v.iter().map(|c| c * c).sum::<f64>().sqrt();
            v.map(|c| c / length)
        };
        let direction = unit(Self::DIRECTION);
        // The sphere just fits the field of view from this distance.
        let distance = self.radius / (FIELD_OF_VIEW_DEGREES / 2.0).to_radians().sin();
        let view_point = [0, 1, 2].map(|axis| self.centre[axis] - direction[axis] * distance);
        Camera {
            projection,
            view_point: vector(view_point),
            direction: vector(direction),
            up_vector: vector(unit(Self::UP)),
            aspect_ratio: (version == Version::V3_0).then_some(ASPECT_RATIO),
        }
    }

    /// Six planes boxing [`Self::bounds`] grown by
    /// [`SECTION_BOX_MARGIN_METRES`], each pointing outwards, since a plane
    /// clips what lies on the side its direction points to: the low then
    /// the high side of x, y and z.
    fn section_box(&self) -> Vec<ClippingPlane> {
        let mut planes = Vec::with_capacity(6);
        for axis in 0..3 {
            for (corner, sign) in [(self.bounds.min, -1.0), (self.bounds.max, 1.0)] {
                let mut location = self.centre;
                location[axis] = corner[axis] + sign * SECTION_BOX_MARGIN_METRES;
                let mut direction = [0.0; 3];
                direction[axis] = sign;
                planes.push(ClippingPlane {
                    location: vector(location),
                    direction: vector(direction),
                });
            }
        }
        planes
    }

    fn perspective(&self, version: Version) -> Camera {
        self.camera(
            Projection::Perspective {
                field_of_view: FIELD_OF_VIEW_DEGREES,
            },
            version,
        )
    }

    fn orthogonal(&self, version: Version) -> Camera {
        self.camera(
            Projection::Orthogonal {
                view_to_world_scale: round(2.0 * self.radius),
            },
            version,
        )
    }
}

fn vector(v: [f64; 3]) -> Vector3 {
    Vector3::new(round(v[0]), round(v[1]), round(v[2]))
}

/// Rounded to micrometres, negative zero made positive.
fn round(value: f64) -> f64 {
    (value * DECIMALS).round() / DECIMALS + 0.0
}

/// The comment a decided finding's topic carries: the decision's status and
/// comment, by its author at its date, then what changed since, if anything.
/// Its GUID derives from the topic's, so re-exporting reproduces it.
fn decision_comment(decision: &FindingDecision, topic: Uuid) -> Comment {
    let status = match decision.status {
        DecisionStatus::Open => "Open",
        DecisionStatus::Accepted => "Accepted",
        DecisionStatus::Rejected => "Rejected",
    };
    let mut text = match decision.comment.trim() {
        "" => status.to_owned(),
        comment => format!("{status}: {comment}"),
    };
    if !decision.changes.is_empty() {
        let changes: Vec<String> = decision
            .changes
            .iter()
            .map(|change| {
                format!(
                    "{} {} -> {}",
                    change.facet.as_str(),
                    change.decided,
                    change.now
                )
            })
            .collect();
        text.push_str("\nChanged since the decision: ");
        text.push_str(&changes.join("; "));
    }
    Comment {
        guid: decision_comment_guid(topic).to_string(),
        date: decision.date.to_string(),
        author: decision.author.trim().to_owned(),
        comment: text,
        viewpoint: None,
    }
}

/// The objects of one entry, subject first, mapped to BCF components.
struct Resolved {
    selection: Vec<Component>,
    unanchored: Vec<ObjectId>,
}

impl Resolved {
    /// `anchored` says whether `objects` starts with the subject. Without one
    /// nothing is selected, and so nothing is reported unanchored either.
    ///
    /// A resource object the report names is never a component: it is no
    /// element a viewer shows, even where it has a GlobalId. It keeps its
    /// topic and is listed unanchored like an object without a GlobalId.
    fn new(
        objects: &[&ObjectId],
        anchored: bool,
        (report, project): (&Report, &Project),
    ) -> Result<Self, ExportError> {
        let mut selection = Vec::new();
        let mut unanchored = Vec::new();
        for (index, id) in objects.iter().enumerate() {
            let object = match project.object(id) {
                Some(object) => Some(object),
                None if report.resource(id).is_some() => None,
                None => return Err(ExportError::UnknownObject((*id).clone())),
            };
            if let Some(global_id) =
                object.and_then(|object| object.external_id(IFC_GLOBAL_ID_SCHEME))
            {
                // A viewpoint of only the related objects would show the
                // reviewer the slab, not the wall that fails to rest on it.
                if anchored && (index == 0 || !selection.is_empty()) {
                    selection.push(Component::ifc(global_id));
                }
            } else if anchored {
                unanchored.push((*id).clone());
            }
        }
        Ok(Self {
            selection,
            unanchored,
        })
    }
}

/// A topic's labels: the rule id, then `Storey: <name>` for each storey and
/// `Space: <name>` for each space the entry is located in (the place's id
/// when it has no name). An unlocated entry has the rule id alone, as
/// before locations existed; the labels never enter the GUID key.
fn labels(rule: &str, location: Option<&Location>) -> Vec<String> {
    let mut labels = vec![rule.to_owned()];
    let named = |place: &Place| place.name.clone().unwrap_or_else(|| place.id.to_string());
    if let Some(location) = location {
        labels.extend(
            location
                .storeys
                .iter()
                .map(|place| format!("Storey: {}", named(place))),
        );
        labels.extend(
            location
                .spaces
                .iter()
                .map(|place| format!("Space: {}", named(place))),
        );
    }
    labels
}

/// Labels a ruleset gives its rules, keyed by rule id as written in it:
/// `Folder: <path>` for a rule inside folders (names joined by ` / `, the
/// root folder left out), then the rule's tags. A rule directly in the root
/// and without tags has none.
///
/// Names and tags are trimmed and blank ones dropped, because BCF refuses
/// labels with surrounding whitespace. A host running several rulesets
/// qualifies the keys as it qualifies the rule ids.
#[must_use]
pub fn ruleset_labels(ruleset: &RuleSetPackage) -> BTreeMap<String, Vec<String>> {
    fn walk(
        folder: &RuleFolder,
        path: &mut Vec<String>,
        labels: &mut BTreeMap<String, Vec<String>>,
    ) {
        for rule in &folder.rules {
            let mut own = Vec::new();
            if !path.is_empty() {
                own.push(format!("Folder: {}", path.join(" / ")));
            }
            own.extend(
                rule.tags
                    .iter()
                    .map(|tag| tag.trim())
                    .filter(|tag| !tag.is_empty())
                    .map(str::to_owned),
            );
            if !own.is_empty() {
                labels.insert(rule.id.clone(), own);
            }
        }
        for child in &folder.folders {
            let name = child.name.default.trim();
            let pushed = !name.is_empty();
            if pushed {
                path.push(name.to_owned());
            }
            walk(child, path, labels);
            if pushed {
                path.pop();
            }
        }
    }
    let mut labels = BTreeMap::new();
    walk(&ruleset.root, &mut Vec::new(), &mut labels);
    labels
}

/// A topic's labels: the rule id, then the host's labels for the rule
/// (folder path, tags), then the finding's category path and location,
/// each once. Labels never enter
/// the GUID key.
fn merged_labels(located: &[String], rule_labels: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    let Some((rule, places)) = located.split_first() else {
        return Vec::new();
    };
    let own = rule_labels.get(rule.as_str()).into_iter().flatten();
    let mut labels: Vec<String> = Vec::new();
    for label in std::iter::once(rule).chain(own).chain(places) {
        if !labels.contains(label) {
            labels.push(label.clone());
        }
    }
    labels
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

/// `Priority` of a finding's topic, from its severity.
fn priority(severity: &Severity) -> &'static str {
    match severity {
        Severity::Error => PRIORITY_HIGH,
        Severity::Warning => PRIORITY_NORMAL,
        Severity::Info => PRIORITY_LOW,
    }
}

fn reason(reason: &NotEvaluatedReason) -> &'static str {
    match reason {
        NotEvaluatedReason::MissingService => "missing service",
        NotEvaluatedReason::BackendUnavailable => "backend unavailable",
        NotEvaluatedReason::IncompleteEvidence => "incomplete evidence",
        NotEvaluatedReason::InvalidEvidence => "invalid evidence",
        NotEvaluatedReason::InvalidDeclaration => "invalid declaration",
        NotEvaluatedReason::UnboundConcept => "unbound concept",
        NotEvaluatedReason::NotRecorded => "not recorded",
        NotEvaluatedReason::ResourceLimit => "resource limit",
    }
}

fn join(ids: &[ObjectId]) -> String {
    ids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
