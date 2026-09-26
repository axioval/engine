//! The wholes an object is part of.
//!
//! An object can be a part of another through aggregation, grouping,
//! spatial containment, nesting, or by filling an opening that voids another
//! element. A requirement that an object be part of a whole of some class
//! needs those wholes in order, nearest first, with the class and predefined
//! type of each; wholes need not themselves be checked objects (an IFC4
//! project is not). Which relationships a source has, and how it chains
//! them, is the source's, so the answer comes from a trusted service.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId, SourceId};
use thiserror::Error;

use crate::{SnapshotBoundService, SourceSnapshot};

/// How a part relates to its whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Decomposition {
    /// The part is an aggregated component; wholes chain upwards.
    Aggregation,
    /// The part is a member of a group; the one group only.
    Grouping,
    /// The part is contained in a spatial structure; the direct container only.
    Containment,
    /// The part is nested in a host; hosts chain upwards.
    Nesting,
    /// The part voids the whole, or fills an opening that voids it.
    Voiding,
    /// Any of these, following the nearest whole of any kind upwards.
    Any,
}

/// One whole, nearest first in a [`ResolvedWholes`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Whole {
    /// The whole's class in the source's vocabulary (IFC: `IFCBUILDINGSTOREY`).
    pub class: String,
    /// The whole's predefined type, resolved as for any object.
    pub predefined_type: Option<String>,
    /// The whole as a project object, when it is one.
    pub object: Option<ObjectId>,
}

/// The wholes of one object under one decomposition, with exact evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedWholes {
    /// Nearest first; empty when the object is part of nothing this way.
    pub wholes: Vec<Whole>,
    /// Where the answer was read.
    pub evidence: Evidence,
}

/// Failure to list an object's wholes conclusively.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum DecompositionError {
    /// The service does not cover this source.
    #[error("decomposition service does not cover source `{0}`")]
    UncoveredSource(SourceId),
    /// The object is not part of the source.
    #[error("object `{0}` is not in the source")]
    UnknownObject(ObjectId),
    /// The object has several wholes where one is expected, so which one a
    /// requirement means is not determined.
    #[error("the whole is ambiguous: {0}")]
    Ambiguous(String),
    /// The source's relationships are malformed or cannot be read exactly.
    #[error("decomposition cannot be read exactly: {0}")]
    Unreadable(String),
}

/// Trusted adapter seam listing the wholes an object is part of.
pub trait DecompositionService: Send + Sync {
    /// Exact source snapshots this service answers for.
    fn source_snapshots(&self) -> &[SourceSnapshot];
    /// The wholes of `object` under `decomposition`, nearest first.
    fn wholes(
        &self,
        object: &ObjectId,
        decomposition: Decomposition,
    ) -> Result<ResolvedWholes, DecompositionError>;
}

/// Cloneable, type-erased decomposition service registered by an adapter.
#[derive(Clone)]
pub struct DecompositionServiceHandle(Arc<dyn DecompositionService>);

impl DecompositionServiceHandle {
    /// Wraps a trusted decomposition service.
    #[must_use]
    pub fn new(service: Arc<dyn DecompositionService>) -> Self {
        Self(service)
    }

    /// Answers for one object, refusing sources the service does not cover
    /// and evidence that is not exact evidence from the object's source.
    pub fn wholes(
        &self,
        object: &ObjectId,
        decomposition: Decomposition,
    ) -> Result<ResolvedWholes, DecompositionError> {
        if !self
            .0
            .source_snapshots()
            .iter()
            .any(|snapshot| *snapshot.source() == object.source)
        {
            return Err(DecompositionError::UncoveredSource(object.source.clone()));
        }
        let resolved = self.0.wholes(object, decomposition)?;
        if resolved.evidence.source != object.source || !resolved.evidence.exact {
            return Err(DecompositionError::Unreadable(
                "decomposition evidence is not exact evidence from the object's source".into(),
            ));
        }
        Ok(resolved)
    }
}

impl SnapshotBoundService for DecompositionServiceHandle {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        self.0.source_snapshots()
    }
}
