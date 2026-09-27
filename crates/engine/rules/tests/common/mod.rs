//! An in-memory exact source for semantic capability tests.
#![allow(dead_code, clippy::match_same_arms)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, CompletePropertyAbsenceEvidence,
    CompleteRelationshipSelection, PropertyEnumeration, PropertyEnumerationRequest,
    PropertyRequest, PropertyResolution, PropertyResolutionError, PropertyResolutionService,
    PropertyResolutionServiceHandle, RelationshipQuery, RelationshipSelectionError,
    RelationshipSelectionRequest, RelationshipSelectionService, RelationshipSelectionServiceHandle,
    ResolvedProperty, RuleCapability, RuleContext, ServiceRegistry, TraversalDirection,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::{Evidence, Object, ObjectId, Project, Property, PropertyValue, RuleId, SourceId};

pub mod doors;
pub mod runtime;

pub fn source() -> SourceId {
    SourceId::new("test", "model").unwrap()
}

pub fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

/// Objects, their exact property values, and directed relationship edges.
pub struct Model {
    objects: Vec<Object>,
    values: BTreeMap<(ObjectId, String, String), PropertyValue>,
    /// relationship -> (relating, related)
    edges: BTreeMap<String, Vec<(ObjectId, ObjectId)>>,
    /// Objects whose properties the source cannot answer.
    unreadable: BTreeSet<ObjectId>,
    /// relationship -> (anchor, locator): further evidence an answer from
    /// that anchor cites.
    citations: BTreeMap<String, Vec<(ObjectId, String)>>,
    /// Whether the source can enumerate an object's properties.
    enumerable: bool,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            objects: Vec::new(),
            values: BTreeMap::new(),
            edges: BTreeMap::new(),
            unreadable: BTreeSet::new(),
            citations: BTreeMap::new(),
            enumerable: true,
        }
    }
}

impl Model {
    /// A source that resolves names but cannot list properties.
    pub fn names_only(mut self) -> Self {
        self.enumerable = false;
        self
    }

    pub fn object(mut self, local: &str, kind: &str) -> Self {
        self.objects.push(Object::new(id(local), kind));
        self
    }

    /// An object of another source document than [`source`].
    pub fn object_in(mut self, document: &str, local: &str, kind: &str) -> Self {
        let source = SourceId::new("test", document).unwrap();
        self.objects
            .push(Object::new(ObjectId::new(source, local).unwrap(), kind));
        self
    }

    pub fn value(mut self, local: &str, set: &str, name: &str, value: PropertyValue) -> Self {
        self.values
            .insert((id(local), set.into(), name.into()), value);
        self
    }

    pub fn text(self, local: &str, set: &str, name: &str, value: &str) -> Self {
        self.value(local, set, name, PropertyValue::String(value.into()))
    }

    pub fn edge(self, relationship: &str, relating: &str, related: &str) -> Self {
        self.edge_between(relationship, id(relating), id(related))
    }

    /// An edge between objects of any source document.
    pub fn edge_between(
        mut self,
        relationship: &str,
        relating: ObjectId,
        related: ObjectId,
    ) -> Self {
        self.edges
            .entry(relationship.into())
            .or_default()
            .push((relating, related));
        self
    }

    /// A value of an object of any source document.
    pub fn value_of(
        mut self,
        object: ObjectId,
        set: &str,
        name: &str,
        value: PropertyValue,
    ) -> Self {
        self.values.insert((object, set.into(), name.into()), value);
        self
    }

    /// Cites `locator` in every answer about `relationship` from `anchor`.
    pub fn cite(mut self, relationship: &str, anchor: &str, locator: &str) -> Self {
        self.citations
            .entry(relationship.into())
            .or_default()
            .push((id(anchor), locator.into()));
        self
    }

    pub fn unreadable(mut self, local: &str) -> Self {
        self.unreadable.insert(id(local));
        self
    }

    pub fn evaluate(
        self,
        capability: &dyn RuleCapability,
        rule: &CompiledRule,
    ) -> CapabilityEvaluation {
        self.evaluate_with(capability, rule, |_| {})
    }

    /// Evaluates with further services registered by `extra`.
    pub fn evaluate_with(
        self,
        capability: &dyn RuleCapability,
        rule: &CompiledRule,
        extra: impl FnOnce(&mut ServiceRegistry),
    ) -> CapabilityEvaluation {
        let project = Project::new(self.objects.clone()).unwrap();
        let shared = Arc::new(self);
        let mut services = ServiceRegistry::new();
        services
            .register(PropertyResolutionServiceHandle::new(shared.clone()))
            .unwrap();
        services
            .register(RelationshipSelectionServiceHandle::new(shared))
            .unwrap();
        extra(&mut services);
        capability.evaluate(
            &RuleContext {
                project: &project,
                services: &services,
            },
            rule,
        )
    }
}

impl PropertyResolutionService for Model {
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        if self.unreadable.contains(request.object_id()) {
            return Err(PropertyResolutionError::Unavailable("unreadable".into()));
        }
        let found = self.values.iter().find(|((object, set, name), _)| {
            object == request.object_id()
                && name == request.property()
                && request.property_set().is_none_or(|wanted| wanted == set)
        });
        match found {
            Some(((object, set, name), value)) => {
                let property = Property::new(set.clone(), name.clone(), value.clone())
                    .unwrap()
                    .with_evidence(Evidence::exact(
                        object.source.clone(),
                        format!("{object}:{set}.{name}"),
                    ));
                Ok(PropertyResolution::Present(ResolvedProperty::try_new(
                    request.clone(),
                    property,
                )?))
            }
            None => Ok(PropertyResolution::Absent(
                CompletePropertyAbsenceEvidence::try_new(
                    request.clone(),
                    Evidence::exact(
                        request.object_id().source.clone(),
                        format!("absent:{}:{}", request.object_id(), request.property()),
                    ),
                )?,
            )),
        }
    }

    fn enumerate(
        &self,
        request: &PropertyEnumerationRequest,
    ) -> Result<PropertyEnumeration, PropertyResolutionError> {
        if self.unreadable.contains(request.object_id()) || !self.enumerable {
            return Err(PropertyResolutionError::Unavailable("unreadable".into()));
        }
        let properties = self
            .values
            .iter()
            .filter(|((object, set, name), _)| {
                object == request.object_id()
                    && request.property_set().matches(set)
                    && request.property().matches(name)
            })
            .map(|((object, set, name), value)| {
                Property::new(set.clone(), name.clone(), value.clone())
                    .unwrap()
                    .with_evidence(Evidence::exact(source(), format!("{object}:{set}.{name}")))
            })
            .collect();
        PropertyEnumeration::try_new(
            request.clone(),
            properties,
            Evidence::exact(source(), format!("enumerated:{}", request.object_id())),
        )
    }
}

impl RelationshipSelectionService for Model {
    fn select(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        let RelationshipQuery::Related {
            relationship,
            direction,
            follow_chain,
        } = request.query()
        else {
            return Err(RelationshipSelectionError::InvalidRequest);
        };
        let Some(edges) = self.edges.get(relationship.as_str()) else {
            return Err(RelationshipSelectionError::Unavailable(format!(
                "unknown relationship {}",
                relationship.as_str()
            )));
        };
        let mut reached = BTreeSet::new();
        let mut queue = vec![request.anchor().clone()];
        let mut seen = BTreeSet::from([request.anchor().clone()]);
        while let Some(current) = queue.pop() {
            for (relating, related) in edges {
                let next = match direction {
                    TraversalDirection::Forward if *relating == current => related,
                    TraversalDirection::Backward if *related == current => relating,
                    TraversalDirection::Either if *relating == current => related,
                    TraversalDirection::Either if *related == current => relating,
                    _ => continue,
                };
                reached.insert(next.clone());
                if *follow_chain && seen.insert(next.clone()) {
                    queue.push(next.clone());
                }
            }
        }
        let candidates = reached
            .into_iter()
            .filter(|candidate| {
                candidate != request.anchor() && request.candidate_universe().contains(candidate)
            })
            .collect();
        let mut evidence = vec![Evidence::exact(
            source(),
            format!("scan:{}", relationship.as_str()),
        )];
        evidence.extend(
            self.citations
                .get(relationship.as_str())
                .into_iter()
                .flatten()
                .filter(|(anchor, _)| anchor == request.anchor())
                .map(|(_, locator)| Evidence::exact(source(), locator.clone())),
        );
        CompleteRelationshipSelection::try_new(request.clone(), candidates, evidence)
    }
}

pub fn rule(
    capability: &str,
    selector: Selector,
    parameters: Vec<(&str, ParameterValue)>,
) -> CompiledRule {
    CompiledRule {
        id: RuleId::new("rule").unwrap(),
        capability: capability.into(),
        severity: Severity::Error,
        selector,
        parameters: parameters
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    }
}

pub fn kind(kind: &str) -> Selector {
    Selector::EntityType {
        object_type: kind.into(),
        include_subtypes: false,
    }
}

pub fn selector(value: Selector) -> ParameterValue {
    ParameterValue::Selector {
        value: Box::new(value),
    }
}

pub fn string(value: &str) -> ParameterValue {
    ParameterValue::String {
        value: value.into(),
    }
}

pub fn strings(values: &[&str]) -> ParameterValue {
    ParameterValue::StringList {
        value: values.iter().map(|value| (*value).to_owned()).collect(),
    }
}

pub fn integer(value: i64) -> ParameterValue {
    ParameterValue::Integer { value }
}

pub fn number(value: f64) -> ParameterValue {
    ParameterValue::Number { value }
}

pub fn boolean(value: bool) -> ParameterValue {
    ParameterValue::Boolean { value }
}

pub fn property(set: Option<&str>, name: &str) -> ParameterValue {
    ParameterValue::PropertyReference {
        property: name.into(),
        property_set: set.map(str::to_owned),
    }
}

/// `(object, message)` of every finding, in emission order.
pub fn findings(evaluation: &CapabilityEvaluation) -> Vec<(String, String)> {
    evaluation
        .findings()
        .iter()
        .map(|finding| (subject(finding), finding.message.clone()))
        .collect()
}

/// A finding's object by local id, or `source` / `project` for a finding
/// about no single object.
pub fn subject(finding: &axioval_ir::Finding) -> String {
    match &finding.scope {
        axioval_ir::Scope::Object(object) => object.local_id.clone(),
        axioval_ir::Scope::Source(_) => "source".to_owned(),
        axioval_ir::Scope::Project => "project".to_owned(),
    }
}

/// Objects of every finding, sorted.
pub fn flagged(evaluation: &CapabilityEvaluation) -> Vec<String> {
    let mut objects: Vec<String> = evaluation.findings().iter().map(subject).collect();
    objects.sort();
    objects
}

/// `(object or "-", reason)` of every not-evaluated outcome.
pub fn unevaluated(
    evaluation: &CapabilityEvaluation,
) -> Vec<(String, axioval_ir::NotEvaluatedReason)> {
    evaluation
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| {
            (
                outcome
                    .object_id()
                    .map_or_else(|| "-".to_owned(), |object| object.local_id.clone()),
                outcome.reason().clone(),
            )
        })
        .collect()
}

/// The relative deviation the finding whose message starts with `message`
/// was graded by, as `(lower, upper)`.
pub fn deviation_of(evaluation: &CapabilityEvaluation, message: &str) -> (f64, f64) {
    let index = evaluation
        .findings()
        .iter()
        .position(|finding| finding.message.starts_with(message))
        .unwrap_or_else(|| panic!("no finding starts with {message:?}"));
    let deviation = evaluation
        .deviation(index)
        .unwrap_or_else(|| panic!("{message:?} is not graded"));
    (deviation.lower(), deviation.upper())
}

/// Asserts `found` holds `expected` within a rounding.
pub fn assert_deviation(found: (f64, f64), expected: (f64, f64)) {
    let near = |a: f64, b: f64| (a - b).abs() <= 1e-9 * b.abs().max(1.0);
    assert!(
        found.0 <= expected.0 && expected.1 <= found.1,
        "{found:?} does not hold {expected:?}"
    );
    assert!(
        near(found.0, expected.0) && near(found.1, expected.1),
        "{found:?} is wider than {expected:?}"
    );
}
