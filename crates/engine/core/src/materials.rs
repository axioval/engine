//! The material an object is made of, as its source assigns it.
//!
//! A source may assign one material, a list, or a composition of layers,
//! profiles or constituents, directly or through the object's type. What a
//! requirement can name is the material's names: the composition's own name,
//! each part's name and category, and each part's material's name and
//! category. The service answers with that set, or with no material at all.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId, SourceId};
use thiserror::Error;

use crate::{SnapshotBoundService, SourceSnapshot};

/// An object's assigned material, as the names it can be identified by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedMaterial {
    /// Every non-empty name and category the assignment states, sorted and
    /// unique. Empty when the material states none.
    pub names: Vec<String>,
    /// Where the assignment was read.
    pub evidence: Evidence,
}

/// Failure to read an object's material conclusively.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum MaterialError {
    /// The service does not cover this source.
    #[error("material service does not cover source `{0}`")]
    UncoveredSource(SourceId),
    /// The object is not part of the source.
    #[error("object `{0}` is not in the source")]
    UnknownObject(ObjectId),
    /// The source's material data cannot be read for this object.
    #[error("material cannot be read exactly: {0}")]
    Unsupported(String),
    /// The source's material data is malformed or ambiguous.
    #[error("material data is malformed: {0}")]
    Unreadable(String),
}

/// Trusted adapter seam reporting an object's assigned material.
pub trait MaterialService: Send + Sync {
    /// Exact source snapshots this service answers for.
    fn source_snapshots(&self) -> &[SourceSnapshot];
    /// The material assigned to `object`, or `None` when it has none.
    fn material(&self, object: &ObjectId) -> Result<Option<ResolvedMaterial>, MaterialError>;
}

/// Cloneable, type-erased material service registered by an adapter.
#[derive(Clone)]
pub struct MaterialServiceHandle(Arc<dyn MaterialService>);

impl MaterialServiceHandle {
    /// Wraps a trusted material service.
    #[must_use]
    pub fn new(service: Arc<dyn MaterialService>) -> Self {
        Self(service)
    }

    /// Answers for one object, refusing sources the service does not cover
    /// and evidence that is not exact evidence from the object's source.
    pub fn material(&self, object: &ObjectId) -> Result<Option<ResolvedMaterial>, MaterialError> {
        if !self
            .0
            .source_snapshots()
            .iter()
            .any(|snapshot| *snapshot.source() == object.source)
        {
            return Err(MaterialError::UncoveredSource(object.source.clone()));
        }
        let resolved = self.0.material(object)?;
        if let Some(material) = &resolved {
            if material.evidence.source != object.source || !material.evidence.exact {
                return Err(MaterialError::Unreadable(
                    "material evidence is not exact evidence from the object's source".into(),
                ));
            }
        }
        Ok(resolved)
    }
}

impl SnapshotBoundService for MaterialServiceHandle {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        self.0.source_snapshots()
    }
}
