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
//! GUIDs of issues that are still there, and a BCF tool can track them.
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
//! # Version
//!
//! BCF 2.1 by default. BCF 3.0 ([`Version::V3_0`]) requires a camera on every
//! viewpoint, so it is written only when every viewpoint has one; otherwise
//! the export is refused with [`ExportError::MissingCamera`].

use std::collections::{BTreeMap, BTreeSet};

use axioval_ir::{
    Finding, Location, NotEvaluated, NotEvaluatedReason, ObjectId, Place, Project, Report, Scope,
    Severity,
};
use openbim_bcf::Component;
use openbim_bcf::write::{
    self, Camera, Document, Projection, TargetVersion, Topic, Vector3, Viewpoint, WriteError,
};
use thiserror::Error;
use uuid::Uuid;

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

/// Namespace of every GUID this crate derives. Changing it changes every GUID.
const NAMESPACE: Uuid = Uuid::from_u128(0x6b1f_5a0e_2c3d_4e8f_9a71_0d2c_5e4b_8f13);

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
    /// they carry no GlobalId alias. Sorted and deduplicated.
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
    let mut unframed = BTreeSet::new();
    let mut topics = Vec::with_capacity(entries.len());
    for (entry, qualified) in entries.iter().zip(qualified) {
        let key = if qualified {
            format!("{}\n{}", entry.key, entry.sources)
        } else {
            entry.key.clone()
        };
        unanchored.extend(entry.unanchored.iter().cloned());
        let (topic, uncamered) = entry.topic(&key, options);
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

/// One report entry, resolved against the project.
struct Entry {
    title: String,
    topic_type: String,
    priority: Option<&'static str>,
    /// The rule id, then the location's storeys and spaces.
    labels: Vec<String>,
    description: String,
    /// GUID input without source qualification.
    key: String,
    /// Every source the entry touches, for disambiguating a repeated key.
    sources: String,
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
    fn finding(finding: &Finding, project: &Project) -> Result<Self, ExportError> {
        let subject = finding.object_id();
        let mut objects: Vec<&ObjectId> = subject.into_iter().collect();
        objects.extend(&finding.related);
        let resolved = Resolved::new(&objects, subject.is_some(), &finding.scope, project)?;
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
            labels: labels(&finding.rule_id.to_string(), finding.location.as_ref()),
            description: description.join("\n"),
            // An object finding's key is unchanged from before scopes
            // existed, so its GUID is too. A scoped one is marked, never
            // named by source: that would change with every file name.
            key: match &finding.scope {
                Scope::Object(_) => format!(
                    "finding\n{}\n{}\n{}",
                    finding.rule_id, resolved.key, finding.message
                ),
                Scope::Source(_) | Scope::Project => format!(
                    "finding\n{}\n{}\n{}\n{}",
                    finding.rule_id,
                    scope_marker(&finding.scope),
                    resolved.key,
                    finding.message
                ),
            },
            sources: resolved.sources,
            selection: resolved.selection,
            framed: objects.into_iter().cloned().collect(),
            unanchored: resolved.unanchored,
        })
    }

    fn not_evaluated(outcome: &NotEvaluated, project: &Project) -> Result<Self, ExportError> {
        let objects: Vec<&ObjectId> = outcome.object_id().into_iter().collect();
        let resolved = Resolved::new(&objects, true, &outcome.scope, project)?;
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
            key: format!(
                "not-evaluated\n{}\n{}\n{reason}\n{}",
                outcome.rule_id, resolved.key, outcome.message
            ),
            sources: resolved.sources,
            selection: resolved.selection,
            framed: objects.into_iter().cloned().collect(),
            unanchored: resolved.unanchored,
        })
    }

    /// The topic, and whether a viewpoint of it has no camera.
    fn topic(&self, key: &str, options: &Options) -> (Topic, Uncamered) {
        let guid = Uuid::new_v5(&NAMESPACE, key.as_bytes());
        let mut uncamered = Uncamered::No;
        let viewpoints = if self.selection.is_empty() {
            vec![]
        } else {
            let viewpoint = |name: &[u8], camera| Viewpoint {
                guid: Uuid::new_v5(&guid, name).to_string(),
                selection: self.selection.clone(),
                camera,
            };
            match self.frame(options.bounds.as_ref()) {
                Ok(frame) => vec![
                    viewpoint(b"viewpoint", Some(frame.perspective(options.version))),
                    viewpoint(
                        b"viewpoint-orthogonal",
                        Some(frame.orthogonal(options.version)),
                    ),
                ],
                Err(why) => {
                    uncamered = why;
                    vec![viewpoint(b"viewpoint", None)]
                }
            }
        };
        let topic = Topic {
            guid: guid.to_string(),
            title: self.title.clone(),
            description: Some(self.description.clone()),
            topic_type: Some(self.topic_type.clone()),
            topic_status: Some(options.status.clone()),
            priority: self.priority.map(str::to_owned),
            labels: self.labels.clone(),
            creation_date: options.date.clone(),
            creation_author: options.author.clone(),
            viewpoints,
            ..Topic::default()
        };
        (topic, uncamered)
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

/// The objects of one entry, subject first, mapped to BCF components.
struct Resolved {
    key: String,
    sources: String,
    selection: Vec<Component>,
    unanchored: Vec<ObjectId>,
}

impl Resolved {
    /// `anchored` says whether `objects` starts with the subject. Without one
    /// nothing is selected, and so nothing is reported unanchored either.
    fn new(
        objects: &[&ObjectId],
        anchored: bool,
        scope: &Scope,
        project: &Project,
    ) -> Result<Self, ExportError> {
        let mut keys = Vec::new();
        let mut sources = BTreeSet::new();
        sources.extend(scope.source().map(ToString::to_string));
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
                if anchored && (index == 0 || !selection.is_empty()) {
                    selection.push(Component::ifc(global_id));
                }
            } else {
                keys.push(id.to_string());
                if anchored {
                    unanchored.push((*id).clone());
                }
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

/// A title the writer accepts: the message, or the rule id when it is blank.
fn title(message: &str, rule: &str) -> String {
    let message = message.trim();
    if message.is_empty() {
        rule.to_owned()
    } else {
        message.to_owned()
    }
}

/// Marks a scoped finding's GUID key apart from any object's.
fn scope_marker(scope: &Scope) -> &'static str {
    match scope {
        Scope::Project => "project",
        Scope::Source(_) => "source",
        Scope::Object(_) => "object",
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
