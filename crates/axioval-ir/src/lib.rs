//! Source-neutral semantic model and declarative package contracts.
#![forbid(unsafe_code)]
#![allow(
    missing_docs,
    clippy::missing_errors_doc,
    clippy::return_self_not_must_use
)]

use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Canonical normalized package contract emitted by `axioval/schema`.
pub mod contract;
pub use contract::{DefinitionPackage, RuleSetPackage};

/// Validation error for source-neutral contracts.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum IrError {
    /// An identity component was blank.
    #[error("{kind} must not be blank")]
    Blank { kind: &'static str },
    /// A project contains an ambiguous identity.
    #[error("duplicate object id: {0}")]
    DuplicateObject(ObjectId),
}

fn required(value: impl Into<String>, kind: &'static str) -> Result<String, IrError> {
    let value = value.into();
    if value.trim().is_empty() {
        Err(IrError::Blank { kind })
    } else {
        Ok(value)
    }
}

/// Stable, source-qualified input identity.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceId {
    pub system: String,
    pub document: String,
}
impl SourceId {
    /// Creates a source identity.
    pub fn new(system: impl Into<String>, document: impl Into<String>) -> Result<Self, IrError> {
        Ok(Self {
            system: required(system, "source system")?,
            document: required(document, "source document")?,
        })
    }
}
impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.system, self.document)
    }
}

/// Stable identity of an object within a source.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectId {
    pub source: SourceId,
    pub local_id: String,
}
impl ObjectId {
    /// Creates a source-qualified object identity.
    pub fn new(source: SourceId, local_id: impl Into<String>) -> Result<Self, IrError> {
        Ok(Self {
            source,
            local_id: required(local_id, "object local id")?,
        })
    }
}
impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.source, self.local_id)
    }
}

/// A value supplied by a source adapter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum PropertyValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Decimal(f64),
    String(String),
}

/// Provenance and exactness of evidence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub source: SourceId,
    pub locator: String,
    pub exact: bool,
}
impl Evidence {
    /// Creates evidence asserted exact by its adapter.
    pub fn exact(source: SourceId, locator: impl Into<String>) -> Self {
        Self {
            source,
            locator: locator.into(),
            exact: true,
        }
    }
}

/// A namespace/code classification.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Classification {
    pub system: String,
    pub code: String,
}
impl Classification {
    /// Creates a classification.
    pub fn new(system: impl Into<String>, code: impl Into<String>) -> Result<Self, IrError> {
        Ok(Self {
            system: required(system, "classification system")?,
            code: required(code, "classification code")?,
        })
    }
}

/// A named semantic property.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Property {
    pub property_set: String,
    pub name: String,
    pub value: PropertyValue,
    pub evidence: Option<Evidence>,
}
impl Property {
    /// Creates a property without provenance.
    pub fn new(
        property_set: impl Into<String>,
        name: impl Into<String>,
        value: PropertyValue,
    ) -> Result<Self, IrError> {
        Ok(Self {
            property_set: required(property_set, "property set")?,
            name: required(name, "property name")?,
            value,
            evidence: None,
        })
    }
    /// Attaches source evidence.
    pub fn with_evidence(mut self, evidence: Evidence) -> Self {
        self.evidence = Some(evidence);
        self
    }
    /// Returns the typed property value.
    pub fn value(&self) -> &PropertyValue {
        &self.value
    }
}

/// A source-neutral object and its semantic facts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    pub id: ObjectId,
    pub kind: String,
    pub properties: Vec<Property>,
    pub classifications: Vec<Classification>,
    pub relationships: BTreeMap<String, Vec<ObjectId>>,
}
impl Object {
    /// Creates an object.
    pub fn new(id: ObjectId, kind: impl Into<String>) -> Self {
        Self {
            id,
            kind: kind.into(),
            properties: vec![],
            classifications: vec![],
            relationships: BTreeMap::new(),
        }
    }
    /// Adds a property.
    pub fn with_property(mut self, property: Property) -> Self {
        self.properties.push(property);
        self
    }
    /// Adds a classification.
    pub fn with_classification(mut self, classification: Classification) -> Self {
        self.classifications.push(classification);
        self
    }
    /// Object semantic kind.
    pub fn kind(&self) -> &str {
        &self.kind
    }
    /// Finds a property by namespace and name.
    pub fn property(&self, set: &str, name: &str) -> Option<&Property> {
        self.properties
            .iter()
            .find(|p| p.property_set == set && p.name == name)
    }
}

/// Deterministically indexed source-neutral project graph.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    objects: BTreeMap<ObjectId, Object>,
}
impl Project {
    /// Builds a project, rejecting ambiguous IDs.
    pub fn new(objects: Vec<Object>) -> Result<Self, IrError> {
        let mut result = Self::default();
        for object in objects {
            if result
                .objects
                .insert(object.id.clone(), object.clone())
                .is_some()
            {
                return Err(IrError::DuplicateObject(object.id));
            }
        }
        Ok(result)
    }
    /// Finds an object by source-qualified ID.
    pub fn object(&self, id: &ObjectId) -> Option<&Object> {
        self.objects.get(id)
    }
    /// Iterates objects in stable identity order.
    pub fn objects(&self) -> impl Iterator<Item = &Object> {
        self.objects.values()
    }
}

/// A source-neutral object selector.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selector {
    pub kinds: Vec<String>,
    pub classification: Option<Classification>,
}
impl Selector {
    /// Selects a semantic kind.
    pub fn by_kind(kind: impl Into<String>) -> Self {
        Self {
            kinds: vec![kind.into()],
            classification: None,
        }
    }
    /// Requires a classification.
    pub fn with_classification(
        mut self,
        system: impl Into<String>,
        code: impl Into<String>,
    ) -> Self {
        self.classification = Classification::new(system, code).ok();
        self
    }
    /// Whether an object matches all selector terms.
    pub fn matches(&self, object: &Object) -> bool {
        (self.kinds.is_empty() || self.kinds.iter().any(|k| k == &object.kind))
            && self
                .classification
                .as_ref()
                .is_none_or(|c| object.classifications.contains(c))
    }
}

/// Stable package-local rule identity.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuleId(String);
impl RuleId {
    /// Creates an ID.
    pub fn new(value: impl Into<String>) -> Result<Self, IrError> {
        Ok(Self(required(value, "rule id")?))
    }
}
impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Severity of a validation finding.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}
/// A deterministic, source-qualified validation outcome.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub rule_id: RuleId,
    pub object_id: ObjectId,
    pub severity: Severity,
    pub message: String,
    pub evidence: Vec<Evidence>,
}
/// Why an object or rule instance could not be evaluated conclusively.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotEvaluatedReason {
    MissingService,
    BackendUnavailable,
    IncompleteEvidence,
    InvalidEvidence,
    InvalidDeclaration,
    ResourceLimit,
}
/// Explicit fail-closed evaluation outcome. This is not a compliance finding.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotEvaluated {
    pub rule_id: RuleId,
    pub object_id: Option<ObjectId>,
    pub reason: NotEvaluatedReason,
    pub message: String,
}
/// Ordered report from a plan execution.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub not_evaluated: Vec<NotEvaluated>,
}
impl Report {
    /// Findings in deterministic order.
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }
    /// Fail-closed rule or object evaluations in deterministic order.
    pub fn not_evaluated(&self) -> &[NotEvaluated] {
        &self.not_evaluated
    }
}
