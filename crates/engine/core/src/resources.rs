//! Resource objects: source instances outside the object population.
//!
//! A project's objects are what a source adapter calls its checked objects
//! (in IFC every occurrence, context and type object). A source holds much
//! more: materials, classifications, relationships, task times, surface
//! styles. A rule may check those as well, but only when it names their
//! class: they are never part of the object population, so a rule selecting
//! every object, an object count, BCF and geometry never see them.
//!
//! The population is separate on both ends. A [`ResourceService`] lists the
//! resource objects of one class of one source on request, so nothing is
//! read for a class no rule names. The runtime asks it, before any rule
//! runs, for every class an entity-type selector of a rule's applicability
//! names, and installs the answers as [`ResourceObjects`]. A class the
//! source declares for its objects, or one with such a subclass when
//! subtypes are included, has no resource objects: a class selects either
//! objects or resource objects, never both, so no existing rule changes.
//!
//! A resource object states its facts through the same services as an
//! object (property, attribute and classification resolution), keyed by its
//! source-qualified identity. It carries no facts of its own.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axioval_ir::contract::Selector;
use axioval_ir::{Object, ObjectId, Project, SourceId};
use thiserror::Error;

use crate::{
    CompiledRule, ConceptBindings, RuleOutcomes, ServiceRegistry, SessionSources,
    SnapshotBoundService, SourceSnapshot,
};

/// The resource objects of one class of one source.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ResourceRequest {
    source: SourceId,
    class: String,
    include_subtypes: bool,
}

impl ResourceRequest {
    /// Asks for the resource objects of `class` in `source`, and of its
    /// subclasses when `include_subtypes` is set.
    ///
    /// # Errors
    ///
    /// [`ResourceError::InvalidRequest`] for a blank class name.
    pub fn try_new(
        source: SourceId,
        class: impl Into<String>,
        include_subtypes: bool,
    ) -> Result<Self, ResourceError> {
        let class = class.into();
        if class.trim().is_empty() {
            return Err(ResourceError::InvalidRequest(
                "a resource class name must not be blank".into(),
            ));
        }
        Ok(Self {
            source,
            class,
            include_subtypes,
        })
    }

    /// The source asked about.
    #[must_use]
    pub fn source(&self) -> &SourceId {
        &self.source
    }

    /// The class, in the source's own vocabulary.
    #[must_use]
    pub fn class(&self) -> &str {
        &self.class
    }

    /// Whether instances of subclasses are asked for too.
    #[must_use]
    pub fn include_subtypes(&self) -> bool {
        self.include_subtypes
    }
}

/// Failure to list resource objects conclusively.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ResourceError {
    /// The service does not cover this source.
    #[error("resource service does not cover source `{0}`")]
    UncoveredSource(SourceId),
    /// The request itself is malformed.
    #[error("invalid resource request: {0}")]
    InvalidRequest(String),
    /// The source cannot be read completely.
    #[error("resource objects cannot be listed exactly: {0}")]
    Unreadable(String),
    /// The service answered with something other than the request's
    /// resource objects.
    #[error("resource service answered out of contract: {0}")]
    InvalidAnswer(String),
}

/// Trusted adapter seam listing a source's resource objects by class.
pub trait ResourceService: Send + Sync {
    /// Exact source snapshots this service answers for.
    fn source_snapshots(&self) -> &[SourceSnapshot];

    /// Every resource object of the request's class, complete or refused,
    /// sorted by identity.
    ///
    /// A class the source does not declare has none. So has a class whose
    /// instances are the source's objects, and, with subtypes, a class one
    /// of whose subclasses is: such a class selects objects only. Every
    /// answered object is of the request's source and carries no properties,
    /// classifications or relationships of its own; its facts are resolved
    /// through the source's services.
    fn resources(&self, request: &ResourceRequest) -> Result<Vec<Object>, ResourceError>;
}

/// Cloneable, type-erased resource service registered by an adapter.
#[derive(Clone)]
pub struct ResourceServiceHandle(Arc<dyn ResourceService>);

impl ResourceServiceHandle {
    /// Wraps a trusted resource service.
    #[must_use]
    pub fn new(service: Arc<dyn ResourceService>) -> Self {
        Self(service)
    }

    /// Answers one request, refusing sources the service does not cover and
    /// answers that break the contract.
    pub fn resources(&self, request: &ResourceRequest) -> Result<Vec<Object>, ResourceError> {
        if !self
            .0
            .source_snapshots()
            .iter()
            .any(|snapshot| *snapshot.source() == request.source)
        {
            return Err(ResourceError::UncoveredSource(request.source.clone()));
        }
        let objects = self.0.resources(request)?;
        for object in &objects {
            if object.id.source != request.source {
                return Err(ResourceError::InvalidAnswer(format!(
                    "{} is not of source `{}`",
                    object.id, request.source
                )));
            }
            if !object.properties.is_empty()
                || !object.classifications.is_empty()
                || !object.relationships.is_empty()
            {
                return Err(ResourceError::InvalidAnswer(format!(
                    "{} carries facts of its own",
                    object.id
                )));
            }
        }
        if let Some(pair) = objects.windows(2).find(|pair| pair[0].id >= pair[1].id) {
            return Err(ResourceError::InvalidAnswer(format!(
                "{} is listed out of order or twice",
                pair[1].id
            )));
        }
        Ok(objects)
    }
}

impl SnapshotBoundService for ResourceServiceHandle {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        self.0.source_snapshots()
    }
}

/// The resource objects one run's rules reach: the answers to every class
/// their applicability selectors name, per source.
///
/// The runtime installs it before any rule runs, replacing any host copy;
/// capabilities read it through [`ResourceObjects::reached`]. Capability
/// tests build one with [`ResourceObjects::with_class`].
#[derive(Clone, Debug, Default)]
pub struct ResourceObjects {
    /// Per source, the class as a selector writes it and whether subtypes
    /// are included: the listed identities, or why they could not be read.
    classes: BTreeMap<(SourceId, String, bool), Result<Vec<ObjectId>, String>>,
    /// The derived groups of each grouping the run derives, by grouping id.
    groups: BTreeMap<String, Listed>,
    objects: BTreeMap<ObjectId, Object>,
}

/// The derived groups of one grouping, and the sources whose groups could
/// not all be listed, with why.
type Listed = (Vec<ObjectId>, Vec<(SourceId, String)>);

/// The resource objects a selector reaches.
#[derive(Debug, Default)]
pub struct Reached<'a> {
    /// Every resource object reached, sorted by identity, once each.
    pub objects: Vec<&'a Object>,
    /// Sources whose resource objects of a reached class could not be
    /// listed, with why, sorted: a selection there is incomplete.
    pub unreadable: Vec<(SourceId, String)>,
}

impl ResourceObjects {
    /// No resource objects.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the answer for `class` (as a selector writes it) in `source`:
    /// its resource objects, or why they could not be listed.
    #[must_use]
    pub fn with_class(
        mut self,
        source: SourceId,
        class: impl Into<String>,
        include_subtypes: bool,
        answer: Result<Vec<Object>, String>,
    ) -> Self {
        let answer = answer.map(|objects| {
            objects
                .into_iter()
                .map(|object| {
                    let id = object.id.clone();
                    self.objects.entry(id.clone()).or_insert(object);
                    id
                })
                .collect()
        });
        self.classes
            .insert((source, class.into(), include_subtypes), answer);
        self
    }

    /// Records the derived groups of the grouping `grouping`, which only a
    /// `derivedGroup` selector naming it reaches, and the sources whose
    /// groups could not all be listed, with why.
    #[must_use]
    pub fn with_groups(
        mut self,
        grouping: impl Into<String>,
        groups: Vec<Object>,
        incomplete: Vec<(SourceId, String)>,
    ) -> Self {
        let ids = groups
            .into_iter()
            .map(|object| {
                let id = object.id.clone();
                self.objects.entry(id.clone()).or_insert(object);
                id
            })
            .collect();
        self.groups.insert(grouping.into(), (ids, incomplete));
        self
    }

    /// Whether no class was answered at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.classes.is_empty() && self.groups.is_empty()
    }

    /// The resource object `id`, if a class listed it.
    #[must_use]
    pub fn object(&self, id: &ObjectId) -> Option<&Object> {
        self.objects.get(id)
    }

    /// The resource objects `selector` reaches, and the sources where it
    /// reaches a class that could not be listed.
    ///
    /// An `entityType` selector reaches its class's resource objects; `allOf`
    /// and `anyOf` reach what any operand reaches; a `ruleOutcome` selector
    /// reaches the resource objects the named rule's outcomes name. Nothing
    /// else reaches a resource object: not `all`, not a negation, not a
    /// related or property selector alone. A reached object is selected only
    /// when the whole selector then matches it.
    #[must_use]
    pub fn reached<'s>(
        &'s self,
        selector: &Selector,
        outcomes: Option<&RuleOutcomes>,
    ) -> Reached<'s> {
        let mut ids = BTreeSet::new();
        let mut unreadable = BTreeSet::new();
        self.reach(selector, outcomes, &mut ids, &mut unreadable);
        Reached {
            objects: ids.iter().filter_map(|id| self.objects.get(*id)).collect(),
            unreadable: unreadable.into_iter().collect(),
        }
    }

    fn reach<'s>(
        &'s self,
        selector: &Selector,
        outcomes: Option<&RuleOutcomes>,
        ids: &mut BTreeSet<&'s ObjectId>,
        unreadable: &mut BTreeSet<(SourceId, String)>,
    ) {
        match selector {
            Selector::EntityType {
                object_type,
                include_subtypes,
            } => {
                for ((source, class, subtypes), answer) in &self.classes {
                    if class != object_type || subtypes != include_subtypes {
                        continue;
                    }
                    match answer {
                        Ok(listed) => ids.extend(listed),
                        Err(why) => {
                            unreadable.insert((source.clone(), why.clone()));
                        }
                    }
                }
            }
            Selector::DerivedGroup { grouping } => {
                if let Some((listed, incomplete)) = self.groups.get(grouping) {
                    ids.extend(listed);
                    unreadable.extend(incomplete.iter().cloned());
                }
            }
            Selector::AllOf { operands } | Selector::AnyOf { operands } => {
                for operand in operands {
                    self.reach(operand, outcomes, ids, unreadable);
                }
            }
            Selector::RuleOutcome { rule, .. } => {
                let Some(record) = outcomes.and_then(|outcomes| outcomes.get(rule)) else {
                    return;
                };
                ids.extend(
                    record
                        .named()
                        .filter_map(|id| self.objects.get_key_value(id).map(|(id, _)| id)),
                );
                unreadable.extend(record.unread_resources().cloned());
            }
            Selector::All
            | Selector::Not { .. }
            | Selector::Related { .. }
            | Selector::Property { .. }
            | Selector::PropertyPattern { .. }
            | Selector::Classification { .. }
            | Selector::DerivedClass { .. }
            | Selector::Discipline { .. }
            | Selector::Source { .. }
            | Selector::Expression { .. }
            | Selector::Objects { .. } => {}
        }
    }
}

/// Every entity-type selector through which `selector` reaches resource
/// objects: its class as written and whether subtypes are included.
fn named_classes<'s>(selector: &'s Selector, out: &mut BTreeSet<(&'s str, bool)>) {
    match selector {
        Selector::EntityType {
            object_type,
            include_subtypes,
        } => {
            out.insert((object_type, *include_subtypes));
        }
        Selector::AllOf { operands } | Selector::AnyOf { operands } => {
            for operand in operands {
                named_classes(operand, out);
            }
        }
        _ => {}
    }
}

/// Installs the run's [`ResourceObjects`]: every class `rules`'
/// applicability selectors name, asked of the registered resource service
/// for every source, each native class once. Without a service, or when no
/// rule names a class, the population is empty. A class a source's
/// vocabulary does not bind reaches nothing there; its objects already
/// report the unbound concept. An answer naming a project object is refused:
/// the two populations never share an identity.
pub(crate) fn install(services: &mut ServiceRegistry, project: &Project, rules: &[CompiledRule]) {
    let mut population = ResourceObjects::new();
    let handle = services.get::<ResourceServiceHandle>().cloned();
    if let Some(handle) = handle {
        let mut named = BTreeSet::new();
        for rule in rules {
            named_classes(&rule.selector, &mut named);
        }
        let sources: Vec<SourceId> = services
            .get::<SessionSources>()
            .map(|sources| sources.iter().cloned().collect())
            .unwrap_or_default();
        let bindings = services.get::<ConceptBindings>();
        let mut asked: BTreeMap<(SourceId, String, bool), Result<Vec<Object>, String>> =
            BTreeMap::new();
        for source in &sources {
            for (written, subtypes) in &named {
                let native = match bindings {
                    Some(bindings) => match bindings.object_type(written, source) {
                        Ok(native) => native.to_owned(),
                        Err(_) => continue,
                    },
                    None => (*written).to_owned(),
                };
                let key = (source.clone(), native.to_ascii_uppercase(), *subtypes);
                let answer = asked
                    .entry(key)
                    .or_insert_with(|| {
                        ResourceRequest::try_new(source.clone(), native, *subtypes)
                            .and_then(|request| handle.resources(&request))
                            .and_then(|objects| {
                                match objects
                                    .iter()
                                    .find(|object| project.object(&object.id).is_some())
                                {
                                    Some(object) => Err(ResourceError::InvalidAnswer(format!(
                                        "{} is an object, not a resource object",
                                        object.id
                                    ))),
                                    None => Ok(objects),
                                }
                            })
                            .map_err(|error| error.to_string())
                    })
                    .clone();
                population = population.with_class(source.clone(), *written, *subtypes, answer);
            }
        }
    }
    services.replace(population);
}
