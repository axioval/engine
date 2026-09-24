//! Source integrity: facts about a source's own consistency, reported as warnings.
//!
//! Rule evaluation answers questions about a model; integrity answers whether
//! the source is shaped the way its own schema says it must be. The two are
//! kept apart on purpose. A source irregularity is not a rule finding (no rule
//! was violated) and not a not-evaluated outcome (nothing was asked), so it
//! gets its own channel. A host shows these as warnings next to the report,
//! and a rule that depends on the affected data either refuses or, when it
//! opted in, skips the instance and cites it.

use std::sync::Arc;

use axioval_ir::{Evidence, SourceId};
use thiserror::Error;

use crate::session::{SnapshotBoundService, SourceSnapshot};

/// How much an integrity issue undermines evidence drawn from the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntegritySeverity {
    /// A known, bounded deviation: rules refuse by default and may opt in to
    /// skip it, e.g. a relationship end the schema requires but real exporters
    /// omit.
    Warning,
    /// The record cannot be read at all, e.g. a reference to a missing
    /// entity. No rule can opt out of this.
    Error,
}

/// One irregularity the source's own schema does not allow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntegrityIssue {
    /// Stable machine-readable code, e.g. `relationship.absent-required-end`.
    pub code: String,
    /// Whether the issue is skippable (warning) or corrupts evidence (error).
    pub severity: IntegritySeverity,
    /// Human-readable description of this occurrence.
    pub message: String,
    /// Exact locator of the offending source record.
    pub evidence: Evidence,
}

/// Failure to produce a complete integrity report.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum IntegrityError {
    /// The service does not cover the requested source.
    #[error("integrity service does not cover source `{0}`")]
    UncoveredSource(SourceId),
    /// An issue was reported without reviewable exact evidence.
    #[error("integrity issue lacks reviewable exact evidence")]
    InexactEvidence,
    /// The service could not complete the scan.
    #[error("integrity scan unavailable: {0}")]
    Unavailable(String),
}

/// Adapter seam listing a source's integrity issues.
pub trait SourceIntegrityService: Send + Sync {
    /// Exact source snapshots the scan covers.
    fn source_snapshots(&self) -> &[SourceSnapshot];
    /// Every issue in one covered source, in a deterministic order.
    fn issues(&self, source: &SourceId) -> Result<Vec<IntegrityIssue>, IntegrityError>;
}

/// Cloneable, type-erased integrity service registered by the host.
#[derive(Clone)]
pub struct SourceIntegrityServiceHandle(Arc<dyn SourceIntegrityService>);

impl SourceIntegrityServiceHandle {
    /// Wraps a trusted integrity service.
    #[must_use]
    pub fn new(service: Arc<dyn SourceIntegrityService>) -> Self {
        Self(service)
    }

    /// Lists issues, validating coverage, evidence exactness and source binding.
    pub fn issues(&self, source: &SourceId) -> Result<Vec<IntegrityIssue>, IntegrityError> {
        if !self
            .0
            .source_snapshots()
            .iter()
            .any(|snapshot| snapshot.source() == source)
        {
            return Err(IntegrityError::UncoveredSource(source.clone()));
        }
        let mut issues = self.0.issues(source)?;
        if issues.iter().any(|issue| {
            !issue.evidence.exact
                || issue.evidence.locator.trim().is_empty()
                || issue.evidence.source != *source
                || issue.code.trim().is_empty()
        }) {
            return Err(IntegrityError::InexactEvidence);
        }
        // Deterministic for hosts and diffs regardless of adapter order.
        issues.sort_by(|left, right| {
            (
                &left.evidence.locator,
                &left.code,
                left.severity,
                &left.message,
            )
                .cmp(&(
                    &right.evidence.locator,
                    &right.code,
                    right.severity,
                    &right.message,
                ))
        });
        issues.dedup();
        Ok(issues)
    }
}

impl SnapshotBoundService for SourceIntegrityServiceHandle {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        self.0.source_snapshots()
    }
}
