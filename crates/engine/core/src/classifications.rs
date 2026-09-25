//! Source-declared classification assignments, for `classification` selectors.
//!
//! A classification selector names a system and a code. Whether an object
//! carries that code is a fact of its source: an IFC file states it through
//! `IfcRelAssociatesClassification` and a chain of classification references,
//! and nothing on the project's objects says whether an adapter looked. So the
//! answer comes from a trusted service, and an object whose source has no such
//! service is not evaluated rather than read as unclassified.

use std::sync::Arc;

use axioval_ir::{ObjectId, SourceId};
use thiserror::Error;

use crate::{SnapshotBoundService, SourceSnapshot};

/// One classification an object carries, directly or inherited from its type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassificationAssignment {
    /// Name of the classification system, or `None` when the source does not
    /// connect this assignment to a system. A selector can then neither
    /// confirm nor rule out a match, and the object is not evaluated.
    pub system: Option<String>,
    /// Codes from the assigned item up to the system root, leaf first. An
    /// entry is `None` when that level states no code.
    pub codes: Vec<Option<String>>,
}

impl ClassificationAssignment {
    /// Whether this assignment is `code` in `system`, or a descendant of it
    /// when `include_descendants` is set. `None` when the system is unknown.
    #[must_use]
    pub fn matches(&self, system: &str, code: &str, include_descendants: bool) -> Option<bool> {
        let own = self.system.as_deref()?;
        if own != system {
            return Some(false);
        }
        let levels = if include_descendants {
            &self.codes[..]
        } else {
            &self.codes[..self.codes.len().min(1)]
        };
        Some(levels.iter().any(|level| level.as_deref() == Some(code)))
    }
}

/// Failure to list an object's classifications conclusively.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ClassificationError {
    /// The service does not cover this source.
    #[error("classification service does not cover source `{0}`")]
    UncoveredSource(SourceId),
    /// The object is not part of the source.
    #[error("object `{0}` is not in the classified source")]
    UnknownObject(ObjectId),
    /// The source's classification data is malformed or cannot be read exactly.
    #[error("classifications cannot be read exactly: {0}")]
    Unreadable(String),
}

/// Trusted adapter seam listing the classifications an object carries.
pub trait ClassificationService: Send + Sync {
    /// Exact source snapshots this service answers for.
    fn source_snapshots(&self) -> &[SourceSnapshot];
    /// Every classification `object` carries, complete or refused.
    fn classifications(
        &self,
        object: &ObjectId,
    ) -> Result<Vec<ClassificationAssignment>, ClassificationError>;
}

/// Cloneable, type-erased classification service registered by an adapter.
#[derive(Clone)]
pub struct ClassificationServiceHandle(Arc<dyn ClassificationService>);

impl ClassificationServiceHandle {
    /// Wraps a trusted classification service.
    #[must_use]
    pub fn new(service: Arc<dyn ClassificationService>) -> Self {
        Self(service)
    }

    /// Answers for one object, refusing sources the service does not cover.
    pub fn classifications(
        &self,
        object: &ObjectId,
    ) -> Result<Vec<ClassificationAssignment>, ClassificationError> {
        if !self
            .0
            .source_snapshots()
            .iter()
            .any(|snapshot| *snapshot.source() == object.source)
        {
            return Err(ClassificationError::UncoveredSource(object.source.clone()));
        }
        self.0.classifications(object)
    }
}

impl SnapshotBoundService for ClassificationServiceHandle {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        self.0.source_snapshots()
    }
}

#[cfg(test)]
mod tests {
    use super::ClassificationAssignment;

    fn chain(system: Option<&str>, codes: &[Option<&str>]) -> ClassificationAssignment {
        ClassificationAssignment {
            system: system.map(str::to_owned),
            codes: codes.iter().map(|code| code.map(str::to_owned)).collect(),
        }
    }

    #[test]
    fn the_leaf_matches_and_ancestors_only_with_descendants() {
        let item = chain(Some("DIN 276"), &[Some("331"), Some("330"), Some("300")]);
        assert_eq!(item.matches("DIN 276", "331", false), Some(true));
        assert_eq!(item.matches("DIN 276", "330", false), Some(false));
        assert_eq!(item.matches("DIN 276", "330", true), Some(true));
        assert_eq!(item.matches("Uniclass", "331", true), Some(false));
    }

    #[test]
    fn an_unknown_system_is_undecided_not_a_mismatch() {
        let item = chain(None, &[Some("331")]);
        assert_eq!(item.matches("DIN 276", "331", false), None);
    }

    #[test]
    fn an_unstated_code_never_matches() {
        let item = chain(Some("DIN 276"), &[None, Some("330")]);
        assert_eq!(item.matches("DIN 276", "331", true), Some(false));
        assert_eq!(item.matches("DIN 276", "330", false), Some(false));
    }
}
