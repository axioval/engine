//! Binding canonical package concepts to the vocabulary of each source.
//!
//! Rule packages name concepts (`axioval:fire.ifc4.wall`), and each concept
//! lists the names it has in external type systems. Sources expose data in
//! their own vocabulary (`IfcWall`, `Pset_WallCommon`). Evaluation therefore
//! translates per source, and only through a name declared for exactly the
//! type system that source's snapshot declares.
//!
//! A concept that cannot bind is never a silent non-match: without this
//! module a wall selector over an IFC model selected nothing and the rule
//! reported a clean pass.

use std::{collections::BTreeMap, sync::Arc};

use axioval_ir::SourceId;
use axioval_ir::contract::ExternalName;
use thiserror::Error;

use crate::session::{SnapshotBoundService, SourceSnapshot};

/// Which catalog a concept belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ConceptKind {
    /// An object type such as a wall.
    ObjectType,
    /// A property such as a fire rating.
    Property,
    /// A property-set qualifier.
    PropertySet,
}

impl std::fmt::Display for ConceptKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ObjectType => "object-type",
            Self::Property => "property",
            Self::PropertySet => "property-set",
        })
    }
}

/// Why a concept could not be bound for one source.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum BindingError {
    /// The concept is not declared by any loaded definition package.
    #[error("unknown {kind} concept `{concept}`")]
    UnknownConcept {
        /// Catalog the concept was looked up in.
        kind: ConceptKind,
        /// Concept identifier.
        concept: String,
    },
    /// The source declares no type system, so no concept can bind to it.
    #[error("source `{0}` declares no type system, so package concepts cannot bind to it")]
    UndeclaredTypeSystem(SourceId),
    /// The concept has no name in any type system the source declared.
    #[error("{kind} concept `{concept}` has no external name in type systems {type_systems:?}")]
    Unbound {
        /// Catalog the concept belongs to.
        kind: ConceptKind,
        /// Concept identifier.
        concept: String,
        /// Type systems the source declared.
        type_systems: Vec<String>,
    },
    /// The concept has different names in several declared type systems.
    ///
    /// Picking one would decide which vocabulary the source "really" uses,
    /// which only the source can say, so the binding is refused.
    #[error("{kind} concept `{concept}` binds ambiguously to {names:?}")]
    Ambiguous {
        /// Catalog the concept belongs to.
        kind: ConceptKind,
        /// Concept identifier.
        concept: String,
        /// Distinct candidate names, sorted.
        names: Vec<String>,
    },
}

/// Canonical concepts of every definition package a ruleset loaded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConceptCatalog {
    entries: BTreeMap<(ConceptKind, String), Vec<ExternalName>>,
}

impl ConceptCatalog {
    /// Adds one concept, refusing a second declaration of the same identity.
    pub(crate) fn insert(
        &mut self,
        kind: ConceptKind,
        concept: &str,
        names: &[ExternalName],
    ) -> Result<(), ()> {
        let key = (kind, concept.to_owned());
        if self.entries.contains_key(&key) {
            return Err(());
        }
        self.entries.insert(key, names.to_vec());
        Ok(())
    }

    /// Whether a concept of this kind is declared.
    #[must_use]
    pub fn contains(&self, kind: ConceptKind, concept: &str) -> bool {
        self.entries.contains_key(&(kind, concept.to_owned()))
    }

    fn names(&self, kind: ConceptKind, concept: &str) -> Result<&[ExternalName], BindingError> {
        self.entries
            .get(&(kind, concept.to_owned()))
            .map(Vec::as_slice)
            .ok_or_else(|| BindingError::UnknownConcept {
                kind,
                concept: concept.to_owned(),
            })
    }
}

/// Per-run translation from package concepts to source names.
///
/// Only the engine constructs this, from the compiled plan and the evidence
/// session's declared type systems, and registers it for the duration of one
/// run. Capabilities find it in the service registry; a capability evaluated
/// directly by a trusted host without it works in raw source vocabulary.
#[derive(Clone, Debug)]
pub struct ConceptBindings {
    catalog: Arc<ConceptCatalog>,
    type_systems: BTreeMap<SourceId, Vec<Arc<str>>>,
}

impl ConceptBindings {
    pub(crate) fn new(
        catalog: Arc<ConceptCatalog>,
        type_systems: BTreeMap<SourceId, Vec<Arc<str>>>,
    ) -> Self {
        Self {
            catalog,
            type_systems,
        }
    }

    /// Source name of an object-type concept.
    pub fn object_type(&self, concept: &str, source: &SourceId) -> Result<&str, BindingError> {
        self.bind(ConceptKind::ObjectType, concept, source)
    }

    /// Source name of a property concept.
    pub fn property(&self, concept: &str, source: &SourceId) -> Result<&str, BindingError> {
        self.bind(ConceptKind::Property, concept, source)
    }

    /// Source name of a property-set concept.
    pub fn property_set(&self, concept: &str, source: &SourceId) -> Result<&str, BindingError> {
        self.bind(ConceptKind::PropertySet, concept, source)
    }

    fn bind(
        &self,
        kind: ConceptKind,
        concept: &str,
        source: &SourceId,
    ) -> Result<&str, BindingError> {
        let names = self.catalog.names(kind, concept)?;
        let declared = self
            .type_systems
            .get(source)
            .filter(|systems| !systems.is_empty())
            .ok_or_else(|| BindingError::UndeclaredTypeSystem(source.clone()))?;
        // The package contract allows at most one name per type system, but a
        // source may declare several systems, so distinct matches are possible.
        let mut matched: Vec<&str> = names
            .iter()
            .filter(|name| declared.iter().any(|system| **system == *name.type_system))
            .map(|name| name.name.as_str())
            .collect();
        matched.sort_unstable();
        matched.dedup();
        match matched.as_slice() {
            [name] => Ok(name),
            [] => Err(BindingError::Unbound {
                kind,
                concept: concept.to_owned(),
                type_systems: declared.iter().map(ToString::to_string).collect(),
            }),
            _ => Err(BindingError::Ambiguous {
                kind,
                concept: concept.to_owned(),
                names: matched.iter().map(ToString::to_string).collect(),
            }),
        }
    }
}

/// Failure to answer a type-hierarchy question conclusively.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum TypeHierarchyError {
    /// The service does not cover this source.
    #[error("type hierarchy does not cover source `{0}`")]
    UncoveredSource(SourceId),
    /// The type is not declared by the source's schema.
    #[error("type `{0}` is not declared by the source schema")]
    UnknownType(String),
}

/// Source-declared subtype relation, used for `includeSubtypes` selection.
pub trait TypeHierarchyService: Send + Sync {
    /// Exact source snapshots this hierarchy answers for.
    fn source_snapshots(&self) -> &[SourceSnapshot];
    /// Whether `kind` is `ancestor` or one of its subtypes.
    fn is_a(&self, kind: &str, ancestor: &str) -> Result<bool, TypeHierarchyError>;
}

/// Cloneable, type-erased type-hierarchy service registered by an adapter.
#[derive(Clone)]
pub struct TypeHierarchyServiceHandle(Arc<dyn TypeHierarchyService>);

impl TypeHierarchyServiceHandle {
    /// Wraps a trusted hierarchy service.
    #[must_use]
    pub fn new(service: Arc<dyn TypeHierarchyService>) -> Self {
        Self(service)
    }

    /// Answers for one object's source, refusing sources the service does not cover.
    pub fn is_a(
        &self,
        source: &SourceId,
        kind: &str,
        ancestor: &str,
    ) -> Result<bool, TypeHierarchyError> {
        if !self
            .0
            .source_snapshots()
            .iter()
            .any(|snapshot| snapshot.source() == source)
        {
            return Err(TypeHierarchyError::UncoveredSource(source.clone()));
        }
        self.0.is_a(kind, ancestor)
    }
}

impl SnapshotBoundService for TypeHierarchyServiceHandle {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        self.0.source_snapshots()
    }
}
