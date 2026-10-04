//! Export profiles: Axioval rule packages written as other formats, with
//! every loss stated.
//!
//! A target format can hold only part of what a rule package expresses. An
//! [`ExportProfile`] writes one format: it takes definition packages and a
//! ruleset and returns an [`ExportOutcome`], the artifact together with the
//! rules it holds and a [`Loss`] for everything it could not hold. Nothing
//! is approximated silently and nothing is dropped silently.
//!
//! # Losses and the safety rule
//!
//! A loss is one of two kinds ([`LossKind`]):
//!
//! - **Refused.** The item's checking meaning cannot be expressed, so the
//!   item is left out of the artifact.
//! - **Degraded.** Structure or presentation is reduced without changing any
//!   verdict: folders flattened, hierarchy depth cut, languages dropped, a
//!   column the target cannot hold, annotations it has no place for.
//!
//! **Anything that would change a check's result is Refused, never
//! Degraded.** A consumer may accept a degraded artifact as checking what
//! the package checks; it must never be misled into running a check that
//! decides differently. A profile that cannot tell whether a reduction
//! changes a verdict refuses.
//!
//! # Judging exactness the same way
//!
//! The [`compare`] module holds what every profile shares to decide that:
//! [`compare::Catalog::canonical`] states what decides a rule, with concepts
//! replaced by the names they bind to and rule ids by position, and
//! [`compare::verdict_difference`] finds where two such statements differ.
//! A profile that reads its artifact back (translating what it wrote into a
//! package again) compares the result with the original this way, and
//! refuses every rule that comes back different. [`precheck`] holds the
//! checks on a rule's gate, grading, severity and target groups that most
//! formats cannot state, and [`precheck::unsupported_expression_node`]
//! names the first node of an expression a profile's
//! [`ExportProfile::expression_kinds`] leave out.
//!
//! # A profile outside this repository
//!
//! A profile is a plain trait object with an open string id, so a host
//! application implements its own format's profile in its own crate,
//! depending on this crate and `axioval-ir` only, and registers it with its
//! own frontend. Nothing here names or enumerates the formats that exist.
#![forbid(unsafe_code)]

use std::fmt;

use axioval_ir::{DefinitionPackage, Report, ReportTable, RuleSetPackage};

pub mod compare;
pub mod precheck;

/// One export target: writes rule packages, and optionally reports,
/// takeoffs and classifications, as one format.
///
/// Only [`ExportProfile::export`] is required; the other exports default
/// to an [unsupported](ExportOutcome::unsupported) outcome.
pub trait ExportProfile {
    /// The profile's id, such as `ids`: an open string a frontend selects
    /// the profile by. Ids are not enumerated anywhere, so a host adds its
    /// own without changing this crate.
    fn id(&self) -> &str;

    /// The format's name as a person reads it, such as `IDS`. Defaults to
    /// the id.
    fn format(&self) -> &str {
        self.id()
    }

    /// The expression node kinds the format can state, as packages write
    /// them (`and`, `compare`, `property`, ...). An expression rule holding
    /// any other node is refused, naming the first one
    /// ([`precheck::unsupported_expression_node`]). Defaults to none: a
    /// profile states no expression unless it says which nodes it can.
    ///
    /// Declaring a kind only says the format may hold such a node; the
    /// profile still judges every rule it writes exactly, as any other.
    fn expression_kinds(&self) -> &[&str] {
        &[]
    }

    /// Writes `ruleset`, whose definitions are among `definitions`.
    ///
    /// Each rule is either among [`ExportOutcome::exported`] or has a
    /// [`LossKind::Refused`] loss whose path is its id; a rule that is
    /// exported may still have [`LossKind::Degraded`] losses.
    fn export(&self, definitions: &[DefinitionPackage], ruleset: &RuleSetPackage) -> ExportOutcome;

    /// Writes a finished report. Unsupported unless the profile says
    /// otherwise.
    fn export_report(&self, report: &Report) -> ExportOutcome {
        let _ = report;
        ExportOutcome::unsupported(self.format(), "report")
    }

    /// Writes takeoff tables. Unsupported unless the profile says
    /// otherwise.
    fn export_takeoff(&self, tables: &[ReportTable]) -> ExportOutcome {
        let _ = tables;
        ExportOutcome::unsupported(self.format(), "takeoff")
    }

    /// Writes the classifications `ruleset` selects by, whose concepts are
    /// among `definitions`. Unsupported unless the profile says otherwise.
    fn export_classification(
        &self,
        definitions: &[DefinitionPackage],
        ruleset: &RuleSetPackage,
    ) -> ExportOutcome {
        let _ = (definitions, ruleset);
        ExportOutcome::unsupported(self.format(), "classification")
    }
}

/// What an export wrote and what it lost.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExportOutcome {
    /// The artifact, `None` when nothing could be written.
    pub artifact: Option<Vec<u8>>,
    /// The ids of the rules the artifact holds, in ruleset order.
    pub exported: Vec<String>,
    /// Everything the artifact does not hold as the package states it, in
    /// ruleset order.
    pub losses: Vec<Loss>,
    /// What the artifact holds, in the format's own terms, such as
    /// `3 specification(s)`; `None` when the format has no such unit.
    pub contents: Option<String>,
}

impl ExportOutcome {
    /// An outcome for an export the profile does not support: no artifact,
    /// and one refused loss at `what`.
    #[must_use]
    pub fn unsupported(format: &str, what: &str) -> Self {
        Self {
            losses: vec![Loss::refused(
                what,
                format!("{format} export writes no {what}"),
            )],
            ..Self::default()
        }
    }

    /// Whether nothing was lost, refused or degraded.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.losses.is_empty()
    }

    /// The refused losses: items the artifact does not hold.
    pub fn refused(&self) -> impl Iterator<Item = &Loss> {
        self.losses
            .iter()
            .filter(|loss| loss.kind == LossKind::Refused)
    }

    /// The degraded losses: structure or presentation the artifact reduced.
    pub fn degraded(&self) -> impl Iterator<Item = &Loss> {
        self.losses
            .iter()
            .filter(|loss| loss.kind == LossKind::Degraded)
    }
}

/// Something an artifact does not hold as the package states it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loss {
    /// What was lost: a rule's id for a rule, otherwise a path into the
    /// package such as `folders/<id>` or `report`.
    pub path: String,
    /// Whether the item was left out or only reduced.
    pub kind: LossKind,
    /// Why, in words.
    pub reason: String,
}

impl Loss {
    /// A loss of checking meaning: the item at `path` is left out.
    #[must_use]
    pub fn refused(path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            kind: LossKind::Refused,
            reason: reason.into(),
        }
    }

    /// A reduction of structure or presentation at `path` that changes no
    /// verdict. Never use it for a difference in what decides a check: that
    /// is [`Loss::refused`].
    #[must_use]
    pub fn degraded(path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            kind: LossKind::Degraded,
            reason: reason.into(),
        }
    }
}

impl fmt::Display for Loss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}: {}", self.path, self.kind, self.reason)
    }
}

/// How much of an item an artifact lost.
///
/// There is no kind for a partial check: an item holds its full checking
/// meaning in the artifact or it is refused. Only structure and
/// presentation may be partial.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LossKind {
    /// The item's checking meaning cannot be expressed; it is left out.
    /// Every difference that would change a check's result is refused.
    Refused,
    /// Structure or presentation is reduced; every verdict is unchanged.
    Degraded,
}

impl fmt::Display for LossKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LossKind::Refused => "refused",
            LossKind::Degraded => "degraded",
        })
    }
}
