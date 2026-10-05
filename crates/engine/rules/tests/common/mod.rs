//! An in-memory exact source for semantic capability tests.
#![allow(dead_code, clippy::match_same_arms)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, CompletePropertyAbsenceEvidence, CompleteRelationshipEdges,
    CompleteRelationshipSelection, PropertyEnumeration, PropertyEnumerationRequest,
    PropertyRequest, PropertyResolution, PropertyResolutionError, PropertyResolutionService,
    PropertyResolutionServiceHandle, RelationshipEdge, RelationshipEdgesRequest, RelationshipQuery,
    RelationshipSelectionError, RelationshipSelectionRequest, RelationshipSelectionService,
    RelationshipSelectionServiceHandle, ResolvedProperty, RuleCapability, RuleContext,
    ServiceRegistry, TraversalDirection,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::{Evidence, Object, ObjectId, Project, Property, PropertyValue, RuleId, SourceId};

pub mod doors;
pub mod expressions;
pub mod runtime;

pub fn source() -> SourceId {
    SourceId::new("test", "model").unwrap()
}

/// Holds a capability rebuilt as a template to the implementation it
/// replaced under the whole outside contract (`Parity::contract()`): the
/// same findings word for word, counts, evidence exactness, related
/// objects and not-evaluated outcomes.
pub fn hold_to_reference(
    capability: &str,
    reference: &CapabilityEvaluation,
    template: &CapabilityEvaluation,
) {
    use axioval_rules::parity::{Observations, Parity};
    let parity = Parity::contract().compare(
        (capability, &Observations::of_evaluation(reference)),
        ("template", &Observations::of_evaluation(template)),
    );
    assert!(parity.holds(), "{}", parity.diff());
}

/// A capability that runs as a template, held to the implementation it
/// replaced on every evaluation: both run on the same context, are held to
/// each other under `Parity::contract()` (`hold_to_reference`), and the
/// template's evaluation is returned. Tests evaluate it in the template's
/// place, so every fixture of the capability is a parity check.
pub struct Held(
    pub &'static (dyn RuleCapability + Sync),
    pub &'static (dyn RuleCapability + Sync),
);

impl RuleCapability for Held {
    fn id(&self) -> &'static str {
        self.0.id()
    }

    fn parameters(&self) -> Vec<axioval_engine::ParameterDescriptor> {
        self.0.parameters()
    }

    fn grades_deviation(&self) -> bool {
        self.0.grades_deviation()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let template = self.0.evaluate(context, rule);
        let reference = self.1.evaluate(context, rule);
        hold_to_reference(self.0.id(), &reference, &template);
        template
    }

    fn template(&self) -> Option<&axioval_engine::template::Template> {
        self.0.template()
    }
}

/// Evaluates `rule` with `property-predicate`, which runs as a template,
/// holding it to the implementation it replaced (`hold_to_reference`).
pub fn predicate(model: Model, rule: &CompiledRule) -> CapabilityEvaluation {
    let template = model
        .clone()
        .evaluate(&axioval_rules::PropertyPredicate, rule);
    let reference = model.evaluate(&axioval_rules::reference::PropertyPredicate, rule);
    hold_to_reference("property-predicate", &reference, &template);
    template
}

pub fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

/// Objects, their exact property values, and directed relationship edges.
#[derive(Clone)]
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
    /// Sets an object carries without any member.
    empty_sets: BTreeSet<(ObjectId, String)>,
    /// Properties stating a value the source cannot read, with their
    /// declared type.
    unreadable_values: BTreeMap<(ObjectId, String, String), String>,
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
            empty_sets: BTreeSet::new(),
            unreadable_values: BTreeMap::new(),
        }
    }
}

impl Model {
    /// A source that resolves names but cannot list properties.
    /// The model's objects as a project.
    #[allow(dead_code)]
    pub fn project(&self) -> axioval_ir::Project {
        axioval_ir::Project::new(self.objects.clone()).unwrap()
    }

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

    /// A value of an object of any source.
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

    /// An edge between objects of any sources.
    pub fn edge_of(mut self, relationship: &str, relating: ObjectId, related: ObjectId) -> Self {
        self.edges
            .entry(relationship.into())
            .or_default()
            .push((relating, related));
        self
    }

    /// A relationship the model answers, with no edges of its own yet.
    pub fn known(mut self, relationship: &str) -> Self {
        self.edges.entry(relationship.into()).or_default();
        self
    }

    /// A set `local` carries without any member.
    pub fn empty_set(mut self, local: &str, set: &str) -> Self {
        self.empty_sets.insert((id(local), set.into()));
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

    /// Cites `locator` in every answer about `relationship` from `anchor`.
    pub fn cite(mut self, relationship: &str, anchor: &str, locator: &str) -> Self {
        self.citations
            .entry(relationship.into())
            .or_default()
            .push((id(anchor), locator.into()));
        self
    }

    /// A property of declared type `data_type` whose stated value the
    /// source cannot read exactly.
    pub fn unreadable_value(mut self, local: &str, set: &str, name: &str, data_type: &str) -> Self {
        self.unreadable_values
            .insert((id(local), set.into(), name.into()), data_type.into());
        self
    }

    pub fn unreadable(mut self, local: &str) -> Self {
        self.unreadable.insert(id(local));
        self
    }

    /// An object of any source whose properties cannot be answered.
    pub fn unreadable_object(mut self, object: ObjectId) -> Self {
        self.unreadable.insert(object);
        self
    }

    /// Gives objects of any source their external identities.
    pub fn with_external_ids(mut self, ids: &[(ObjectId, axioval_ir::ExternalId)]) -> Self {
        for (object, external) in ids {
            let held = self
                .objects
                .iter_mut()
                .find(|held| &held.id == object)
                .expect("the object is in the model");
            *held = held.clone().with_external_id(external.clone());
        }
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
    /// The model's project and its property and relationship services.
    #[allow(dead_code)]
    pub fn services(self) -> (Project, ServiceRegistry) {
        let project = Project::new(self.objects.clone()).unwrap();
        let shared = Arc::new(self);
        let mut services = ServiceRegistry::new();
        services
            .register(PropertyResolutionServiceHandle::new(shared.clone()))
            .unwrap();
        services
            .register(RelationshipSelectionServiceHandle::new(shared))
            .unwrap();
        (project, services)
    }

    /// Evaluates as [`Self::evaluate_with`] does, the measured set answered
    /// through the registered measured values as a run answers it.
    #[allow(dead_code)]
    pub fn evaluate_measured(
        self,
        capability: &dyn RuleCapability,
        rule: &CompiledRule,
        extra: impl Fn(&mut ServiceRegistry),
    ) -> CapabilityEvaluation {
        let project = Project::new(self.objects.clone()).unwrap();
        let shared = Arc::new(self);
        let registry =
            axioval_rules::register_builtins(axioval_engine::CapabilityRegistry::new()).unwrap();
        // What the measured values read: the model and the geometry.
        let mut inner = ServiceRegistry::new();
        inner
            .register(PropertyResolutionServiceHandle::new(shared.clone()))
            .unwrap();
        inner
            .register(RelationshipSelectionServiceHandle::new(shared.clone()))
            .unwrap();
        extra(&mut inner);
        registry.install_measured(&mut inner, &project);
        // A template reads a measured value directly, as a run reads it.
        let values = axioval_engine::MeasuredValues::of(&inner, &project);
        // What the capability reads: the same, the measured set answered.
        let mut services = ServiceRegistry::new();
        services
            .register(PropertyResolutionServiceHandle::new(Arc::new(Measuring {
                model: shared.clone(),
                services: inner,
                project: project.clone(),
            })))
            .unwrap();
        services
            .register(RelationshipSelectionServiceHandle::new(shared))
            .unwrap();
        extra(&mut services);
        // Aggregates over measured members ask the providers directly.
        registry.install_measured(&mut services, &project);
        services.register(values).unwrap();
        capability.evaluate(
            &RuleContext {
                project: &project,
                services: &services,
            },
            rule,
        )
    }

    /// Evaluates `template` as a run does (the measured set answered) and
    /// `reference`, the implementation it replaced, on the same model and
    /// services; asserts the template keeps the reference's whole outside
    /// contract (`Parity::contract()`), each report-table value of
    /// `values` within its step and graded deviations within `deviation`
    /// (0 allows a few units in the last place); and returns the
    /// template's evaluation.
    #[allow(dead_code)]
    pub fn holding_contract(
        self,
        template: &dyn RuleCapability,
        reference: &dyn RuleCapability,
        rule: &CompiledRule,
        extra: impl Fn(&mut ServiceRegistry),
        values: &[(&str, f64)],
        deviation: f64,
    ) -> CapabilityEvaluation {
        use axioval_rules::parity::{Observations, Parity};
        let evaluated = self.clone().evaluate_measured(template, rule, &extra);
        let replaced = self.evaluate_with(reference, rule, &extra);
        let mut parity = values
            .iter()
            .fold(Parity::contract(), |parity, (name, step)| {
                parity.value(*name, *step)
            });
        parity.deviations = Some(deviation);
        let parity = parity.compare(
            (reference.id(), &Observations::of_evaluation(&replaced)),
            ("template", &Observations::of_evaluation(&evaluated)),
        );
        assert!(parity.holds(), "{}", parity.diff());
        evaluated
    }

    /// The measured value `name` of each of `objects` as a run reads it,
    /// with `extra`'s geometry: what a rewrite or template compares, for
    /// the parity harness's value comparison. Stated absent is `null`, a
    /// refusal not evaluated.
    pub fn measure(
        self,
        name: &str,
        objects: &[ObjectId],
        extra: impl Fn(&mut ServiceRegistry),
    ) -> Vec<(ObjectId, axioval_rules::parity::Measure)> {
        use axioval_rules::parity::Measure;
        let project = Project::new(self.objects.clone()).unwrap();
        let shared = Arc::new(self);
        let registry =
            axioval_rules::register_builtins(axioval_engine::CapabilityRegistry::new()).unwrap();
        let mut services = ServiceRegistry::new();
        services
            .register(PropertyResolutionServiceHandle::new(shared.clone()))
            .unwrap();
        services
            .register(RelationshipSelectionServiceHandle::new(shared))
            .unwrap();
        extra(&mut services);
        registry.install_measured(&mut services, &project);
        objects
            .iter()
            .map(|object| {
                let value = match axioval_engine::measured_value(&services, &project, object, name)
                {
                    Ok(PropertyResolution::Present(resolved)) => {
                        Measure::of_property(&resolved.property().value)
                            .unwrap_or(Measure::NotEvaluated)
                    }
                    Ok(PropertyResolution::Absent(_)) => Measure::Null,
                    Err(_) => Measure::NotEvaluated,
                };
                (object.clone(), value)
            })
            .collect()
    }

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
        let unreadable = self
            .unreadable_values
            .iter()
            .find(|((object, set, name), _)| {
                object == request.object_id()
                    && name == request.property()
                    && request.property_set().is_none_or(|wanted| wanted == set)
            });
        if let Some(((object, set, name), data_type)) = unreadable {
            return Err(PropertyResolutionError::UnreadableValue(Box::new(
                axioval_engine::UnreadableValue::try_new(
                    request.clone(),
                    data_type.clone(),
                    Evidence::exact(object.source.clone(), format!("{object}:{set}.{name}")),
                    format!("the unit of {name} is unknown"),
                )?,
            )));
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
        if self.unreadable.contains(request.object_id())
            || !self.enumerable
            || self
                .unreadable_values
                .keys()
                .any(|(object, _, _)| object == request.object_id())
        {
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
                    .with_evidence(Evidence::exact(
                        object.source.clone(),
                        format!("{object}:{set}.{name}"),
                    ))
            })
            .collect();
        let empty = self
            .empty_sets
            .iter()
            .filter(|(object, set)| {
                object == request.object_id() && request.property_set().matches(set)
            })
            .map(|(_, set)| set.clone());
        PropertyEnumeration::try_new(
            request.clone(),
            properties,
            Evidence::exact(
                request.object_id().source.clone(),
                format!("enumerated:{}", request.object_id()),
            ),
        )?
        .with_empty_sets(empty)
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

    fn edges(
        &self,
        request: &RelationshipEdgesRequest,
    ) -> Result<CompleteRelationshipEdges, RelationshipSelectionError> {
        let relationship = request.relationship().as_str();
        let Some(edges) = self.edges.get(relationship) else {
            return Err(RelationshipSelectionError::Unavailable(format!(
                "unknown relationship {relationship}"
            )));
        };
        let held = |object: &ObjectId| request.universe().binary_search(object).is_ok();
        let listed: BTreeSet<RelationshipEdge> = edges
            .iter()
            .filter(|(relating, related)| held(relating) && held(related))
            .map(|(relating, related)| RelationshipEdge {
                relating: relating.clone(),
                related: related.clone(),
            })
            .collect();
        CompleteRelationshipEdges::try_new(
            request.clone(),
            listed.into_iter().collect(),
            vec![Evidence::exact(source(), format!("scan:{relationship}"))],
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

/// A parameter given as an expression, written in its package form.
#[allow(dead_code)]
pub fn expression(value: serde_json::Value) -> ParameterValue {
    ParameterValue::Expression {
        value: Box::new(serde_json::from_value(value).unwrap()),
    }
}

/// The measured value `name` of `object` with `services`, as the interval
/// `(lower, upper)` it lies in: `None` when it is absent, `Err` when it
/// cannot be measured. Built-in providers are installed as a run would.
#[allow(dead_code)]
pub fn measured(
    services: &axioval_engine::ServiceRegistry,
    project: &axioval_ir::Project,
    object: &ObjectId,
    name: &str,
) -> Result<Option<(f64, f64)>, String> {
    use axioval_engine::{CapabilityRegistry, PropertyResolution, measured_value};
    let mut services = services.clone();
    axioval_rules::register_builtins(CapabilityRegistry::new())
        .unwrap()
        .install_measured(&mut services, project);
    match measured_value(&services, project, object, name) {
        Ok(PropertyResolution::Present(resolved)) => Ok(Some(match resolved.property().value() {
            PropertyValue::Quantity { value, .. } | PropertyValue::Decimal(value) => {
                (*value, *value)
            }
            #[allow(clippy::cast_precision_loss)]
            PropertyValue::Integer(value) => (*value as f64, *value as f64),
            PropertyValue::Measured { lower, upper, .. } => (*lower, *upper),
            other => return Err(format!("{name} of {object} is {other:?}")),
        })),
        Ok(PropertyResolution::Absent(_)) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// A measured interval and whether its evidence is exact.
pub type Cited = ((f64, f64), bool);

/// [`measured`], with whether the value's evidence is exact.
#[allow(dead_code)]
pub fn measured_cited(
    services: &axioval_engine::ServiceRegistry,
    project: &axioval_ir::Project,
    object: &ObjectId,
    name: &str,
) -> Result<Option<Cited>, String> {
    use axioval_engine::{CapabilityRegistry, PropertyResolution, measured_value};
    let mut installed = services.clone();
    axioval_rules::register_builtins(CapabilityRegistry::new())
        .unwrap()
        .install_measured(&mut installed, project);
    let exact = match measured_value(&installed, project, object, name) {
        Ok(PropertyResolution::Present(resolved)) => resolved
            .property()
            .evidence
            .as_ref()
            .is_some_and(|evidence| evidence.exact),
        Ok(PropertyResolution::Absent(_)) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    measured(services, project, object, name).map(|value| value.map(|value| (value, exact)))
}

/// Whether `value` is surely at least `bound` (`Some(true)`), surely below
/// it (`Some(false)`), or straddles it (`None`).
#[allow(dead_code)]
pub fn at_least((lower, upper): (f64, f64), bound: f64) -> Option<bool> {
    if lower >= bound {
        Some(true)
    } else if upper < bound {
        Some(false)
    } else {
        None
    }
}

/// The model's properties, the measured set answered as a run answers it.
struct Measuring {
    model: Arc<Model>,
    services: ServiceRegistry,
    project: Project,
}

impl PropertyResolutionService for Measuring {
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        if request.property_set() == Some(axioval_ir::MEASURED_SET) {
            return axioval_engine::measured_value(
                &self.services,
                &self.project,
                request.object_id(),
                request.property(),
            );
        }
        self.model.resolve(request)
    }

    fn enumerate(
        &self,
        request: &axioval_engine::PropertyEnumerationRequest,
    ) -> Result<axioval_engine::PropertyEnumeration, PropertyResolutionError> {
        self.model.enumerate(request)
    }
}
