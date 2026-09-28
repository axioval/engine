//! Stable identities of findings and not-evaluated outcomes.
//!
//! A reviewer's decision about a finding must survive re-checking a revised
//! model, so a finding needs an identity that does not depend on how the
//! source numbers its objects. The identity is a UUIDv5 over the rule, the
//! identities of the finding's objects in a *stable scheme* the host names
//! (an [`ExternalId`](crate::ExternalId) scheme whose values survive a
//! re-export, such as a globally unique id the authoring tool keeps), and the
//! message. An object without an alias in that scheme falls back to its
//! source-qualified [`ObjectId`], which is only as stable as the source's
//! numbering.
//!
//! The key layout and `NAMESPACE` are a compatibility contract: every
//! identity a user has ever recorded a decision against, and every BCF topic
//! GUID ever exported, is derived from them.
//!
//! Locations, evidence and severity never enter the key: a finding keeps its
//! identity whether or not it was located, and a finding regraded by a
//! revised model is the same finding with changed evidence.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{NotEvaluated, NotEvaluatedReason, ObjectId, Project, Report, Scope};

/// Namespace of every identity derived here. Changing it changes every
/// identity and every BCF topic GUID.
pub const NAMESPACE: Uuid = Uuid::from_u128(0x6b1f_5a0e_2c3d_4e8f_9a71_0d2c_5e4b_8f13);

/// The stable identity of one finding: a `UUIDv5`, written in its hyphenated
/// lowercase form (`8c2f…-…`).
///
/// Equal for the same rule, the same objects (by their stable aliases) and
/// the same message, so re-checking a revised model reproduces the identity
/// of every finding still present.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FindingId(Uuid);

impl FindingId {
    /// The identity as a UUID.
    #[must_use]
    pub fn uuid(self) -> Uuid {
        self.0
    }
}

impl From<Uuid> for FindingId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl fmt::Display for FindingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(f)
    }
}

impl FromStr for FindingId {
    type Err = IdentityError;
    fn from_str(text: &str) -> Result<Self, IdentityError> {
        Uuid::try_parse(text)
            .map(Self)
            .map_err(|_| IdentityError::NotAnIdentity(text.to_owned()))
    }
}

/// Why identities could not be derived or read.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum IdentityError {
    /// The report names an object neither the project nor the report's
    /// resource objects contain, so it was not computed over this project.
    #[error("report names {0}, which is not in the project")]
    UnknownObject(ObjectId),
    /// The text is not a UUID.
    #[error("`{0}` is not a finding identity (a UUID)")]
    NotAnIdentity(String),
}

/// The identity of every finding of `report`, in report order.
///
/// `stable_scheme` is the external id scheme whose values survive a
/// re-export. A key made only of stable aliases can repeat across sources,
/// such as two revisions of one model federated in one project; those keys,
/// and only those, are qualified by the sources the finding touches, so every
/// identity in one report is distinct.
///
/// # Errors
///
/// [`IdentityError::UnknownObject`] when a finding names an object the
/// project does not contain.
pub fn finding_ids(
    report: &Report,
    project: &Project,
    stable_scheme: &str,
) -> Result<Vec<FindingId>, IdentityError> {
    let keys = report
        .findings()
        .iter()
        .map(|finding| {
            let subject = finding.object_id();
            let mut objects: Vec<&ObjectId> = subject.into_iter().collect();
            objects.extend(&finding.related);
            let resolved =
                Resolved::new(&objects, &finding.scope, (report, project), stable_scheme)?;
            // An object finding's key has no scope marker, as before scopes
            // existed. A scoped one is marked, never named by source: that
            // would change with every file name.
            let key = match &finding.scope {
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
            };
            Ok((key, resolved.sources))
        })
        .collect::<Result<Vec<_>, IdentityError>>()?;
    Ok(qualified(&keys).into_iter().map(FindingId).collect())
}

/// The identity of every not-evaluated outcome of `report`, in report
/// order, derived as [`finding_ids`] derives a finding's. Never equal to a
/// finding's: the key names what kind of outcome it is.
///
/// # Errors
///
/// [`IdentityError::UnknownObject`] when an outcome names an object the
/// project does not contain.
pub fn not_evaluated_ids(
    report: &Report,
    project: &Project,
    stable_scheme: &str,
) -> Result<Vec<Uuid>, IdentityError> {
    let keys = report
        .not_evaluated()
        .iter()
        .map(|outcome: &NotEvaluated| {
            let objects: Vec<&ObjectId> = outcome.object_id().into_iter().collect();
            let resolved =
                Resolved::new(&objects, &outcome.scope, (report, project), stable_scheme)?;
            let key = format!(
                "not-evaluated\n{}\n{}\n{}\n{}",
                outcome.rule_id,
                resolved.key,
                reason(&outcome.reason),
                outcome.message
            );
            Ok((key, resolved.sources))
        })
        .collect::<Result<Vec<_>, IdentityError>>()?;
    Ok(qualified(&keys))
}

/// `UUIDv5` of each key, qualified by its sources when the key repeats.
fn qualified(keys: &[(String, String)]) -> Vec<Uuid> {
    let mut uses: BTreeMap<&str, usize> = BTreeMap::new();
    for (key, _) in keys {
        *uses.entry(key.as_str()).or_default() += 1;
    }
    keys.iter()
        .map(|(key, sources)| {
            let key = if uses[key.as_str()] > 1 {
                format!("{key}\n{sources}")
            } else {
                key.clone()
            };
            Uuid::new_v5(&NAMESPACE, key.as_bytes())
        })
        .collect()
}

/// The objects of one outcome, subject first, as key text.
struct Resolved {
    key: String,
    sources: String,
}

impl Resolved {
    fn new(
        objects: &[&ObjectId],
        scope: &Scope,
        (report, project): (&Report, &Project),
        stable_scheme: &str,
    ) -> Result<Self, IdentityError> {
        let mut keys = Vec::with_capacity(objects.len());
        let mut sources = BTreeSet::new();
        sources.extend(scope.source().map(ToString::to_string));
        for id in objects {
            // A resource object is keyed like an object: by its stable
            // alias where it has one, else by its identity.
            let object = report
                .object(project, id)
                .ok_or_else(|| IdentityError::UnknownObject((*id).clone()))?;
            sources.insert(id.source.to_string());
            keys.push(
                object
                    .external_id(stable_scheme)
                    .map_or_else(|| id.to_string(), ToOwned::to_owned),
            );
        }
        Ok(Self {
            key: keys.join("\n"),
            sources: sources.into_iter().collect::<Vec<_>>().join("\n"),
        })
    }
}

/// Marks a scoped finding's key apart from any object's.
fn scope_marker(scope: &Scope) -> &'static str {
    match scope {
        Scope::Project => "project",
        Scope::Source(_) => "source",
        Scope::Object(_) => "object",
    }
}

/// The reason as the key spells it.
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
