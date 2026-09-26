//! An in-memory exact source for semantic capability tests.
#![allow(dead_code, clippy::match_same_arms)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, CompletePropertyAbsenceEvidence,
    CompleteRelationshipSelection, PropertyRequest, PropertyResolution, PropertyResolutionError,
    PropertyResolutionService, PropertyResolutionServiceHandle, RelationshipQuery,
    RelationshipSelectionError, RelationshipSelectionRequest, RelationshipSelectionService,
    RelationshipSelectionServiceHandle, ResolvedProperty, RuleCapability, RuleContext,
    ServiceRegistry, TraversalDirection,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::{Evidence, Object, ObjectId, Project, Property, PropertyValue, RuleId, SourceId};

pub fn source() -> SourceId {
    SourceId::new("test", "model").unwrap()
}

pub fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

/// Objects, their exact property values, and directed relationship edges.
#[derive(Default)]
pub struct Model {
    objects: Vec<Object>,
    values: BTreeMap<(ObjectId, String, String), PropertyValue>,
    /// relationship -> (relating, related)
    edges: BTreeMap<String, Vec<(ObjectId, ObjectId)>>,
    /// Objects whose properties the source cannot answer.
    unreadable: BTreeSet<ObjectId>,
}

impl Model {
    pub fn object(mut self, local: &str, kind: &str) -> Self {
        self.objects.push(Object::new(id(local), kind));
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

    pub fn edge(mut self, relationship: &str, relating: &str, related: &str) -> Self {
        self.edges
            .entry(relationship.into())
            .or_default()
            .push((id(relating), id(related)));
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
        let project = Project::new(self.objects.clone()).unwrap();
        let shared = Arc::new(self);
        let mut services = ServiceRegistry::new();
        services
            .register(PropertyResolutionServiceHandle::new(shared.clone()))
            .unwrap();
        services
            .register(RelationshipSelectionServiceHandle::new(shared))
            .unwrap();
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
                    .with_evidence(Evidence::exact(source(), format!("{object}:{set}.{name}")));
                Ok(PropertyResolution::Present(ResolvedProperty::try_new(
                    request.clone(),
                    property,
                )?))
            }
            None => Ok(PropertyResolution::Absent(
                CompletePropertyAbsenceEvidence::try_new(
                    request.clone(),
                    Evidence::exact(
                        source(),
                        format!("absent:{}:{}", request.object_id(), request.property()),
                    ),
                )?,
            )),
        }
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
        CompleteRelationshipSelection::try_new(
            request.clone(),
            candidates,
            vec![Evidence::exact(
                source(),
                format!("scan:{}", relationship.as_str()),
            )],
        )
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
        .map(|finding| (finding.object_id.local_id.clone(), finding.message.clone()))
        .collect()
}

/// Objects of every finding, sorted.
pub fn flagged(evaluation: &CapabilityEvaluation) -> Vec<String> {
    let mut objects: Vec<String> = evaluation
        .findings()
        .iter()
        .map(|finding| finding.object_id.local_id.clone())
        .collect();
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
