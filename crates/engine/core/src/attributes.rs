//! An object's own attributes, as its source defines them.
//!
//! Attributes are the fields a source's schema gives an object class
//! directly, such as an IFC element's `Name`, `Description` or `Tag`, as
//! opposed to properties grouped into sets. Which attributes a class has, and
//! what an unset one looks like, is a fact of the source, so the answer comes
//! from a trusted service, and an attribute the class does not have is an
//! error rather than an absence.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId, PropertyValue, SourceId};
use thiserror::Error;

use crate::{SnapshotBoundService, SourceSnapshot};

/// What an attribute holds.
#[derive(Clone, Debug, PartialEq)]
pub enum AttributeValue {
    /// No value: the attribute is unset, an empty aggregate, or a logical
    /// that is neither true nor false.
    Unset,
    /// A scalar, with the type the source declares for it when it has one
    /// (an IFC `IfcLabel` attribute: `IFCLABEL`; an enumeration: its type).
    Scalar {
        /// The value; enumeration items are their text.
        value: PropertyValue,
        /// The source-declared type, in the source's own vocabulary.
        data_type: Option<String>,
    },
    /// A value that is present but not a scalar: a reference to another
    /// object or a non-empty aggregate. It has no value to compare.
    Structured,
}

/// One attribute of one object, with exact evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedAttribute {
    /// What it holds.
    pub value: AttributeValue,
    /// Where it was read.
    pub evidence: Evidence,
}

/// Failure to read an attribute conclusively.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum AttributeError {
    /// The service does not cover this source.
    #[error("attribute service does not cover source `{0}`")]
    UncoveredSource(SourceId),
    /// The object is not part of the source.
    #[error("object `{0}` is not in the source")]
    UnknownObject(ObjectId),
    /// The object's class has no attribute of this name.
    #[error("`{class}` has no attribute `{attribute}`")]
    UnknownAttribute {
        /// The object's class.
        class: String,
        /// The requested name.
        attribute: String,
    },
    /// The value exists but cannot be represented exactly, such as a
    /// derived attribute or a measure whose unit is not read.
    #[error("attribute cannot be read exactly: {0}")]
    Unsupported(String),
    /// The source's data is malformed.
    #[error("attribute data is malformed: {0}")]
    Unreadable(String),
}

/// Trusted adapter seam reading an object's own attributes.
pub trait AttributeService: Send + Sync {
    /// Exact source snapshots this service answers for.
    fn source_snapshots(&self) -> &[SourceSnapshot];
    /// The attribute `name` of `object`, in the source's own vocabulary.
    fn attribute(&self, object: &ObjectId, name: &str)
    -> Result<ResolvedAttribute, AttributeError>;
}

/// Cloneable, type-erased attribute service registered by an adapter.
#[derive(Clone)]
pub struct AttributeServiceHandle(Arc<dyn AttributeService>);

impl AttributeServiceHandle {
    /// Wraps a trusted attribute service.
    #[must_use]
    pub fn new(service: Arc<dyn AttributeService>) -> Self {
        Self(service)
    }

    /// Answers for one object, refusing sources the service does not cover
    /// and evidence from any other source.
    pub fn attribute(
        &self,
        object: &ObjectId,
        name: &str,
    ) -> Result<ResolvedAttribute, AttributeError> {
        if !self
            .0
            .source_snapshots()
            .iter()
            .any(|snapshot| *snapshot.source() == object.source)
        {
            return Err(AttributeError::UncoveredSource(object.source.clone()));
        }
        let resolved = self.0.attribute(object, name)?;
        if resolved.evidence.source != object.source || !resolved.evidence.exact {
            return Err(AttributeError::Unreadable(
                "attribute evidence is not exact evidence from the object's source".into(),
            ));
        }
        Ok(resolved)
    }
}

impl SnapshotBoundService for AttributeServiceHandle {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        self.0.source_snapshots()
    }
}
