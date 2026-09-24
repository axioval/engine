//! Exact relationship selection over the objectified relationships of one IFC4 model.
//!
//! IFC stores no parent pointers: an `IfcRelContainedInSpatialStructure`
//! entity names both the storey and its elements. A relationship identity in a
//! request is therefore the IFC4 name of a relationship entity type, and every
//! instance of that type (or of a subtype) is one edge set from its relating
//! end to each of its related ends.
//!
//! Which attribute slot is which end is read from the bundled normative
//! schema rather than hard-coded: the slot positions differ per type
//! (`IfcRelAggregates` keeps the relating end in slot 4, containment in slot
//! 5, `IfcRelConnectsElements` in slot 5 because connection geometry takes 4),
//! and a hand-written table silently inverts direction when one is wrong.
//!
//! Every answer is complete for its source or refused. A malformed instance, a
//! reference to a missing entity, a relationship type whose ends are not plain
//! object references, or a candidate from another source makes the whole
//! answer `Unavailable`: leaving one edge out would turn "not related" into a
//! claim the file does not make.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

use axioval_engine::{
    AbsentEndPolicy, CompleteRelationshipSelection, RelationshipQuery, RelationshipSelectionError,
    RelationshipSelectionRequest, RelationshipSelectionService, SourceSnapshot, TraversalDirection,
};
use axioval_ir::{Evidence, ObjectId};
use ifc_model::{EntityId, Model, Value};
use ifc_schema::{Schema, TypeKind, ifc4};

/// End slots of one concrete relationship entity type.
#[derive(Clone, Debug)]
pub(crate) struct Ends {
    relating: usize,
    relating_optional: bool,
    relating_name: String,
    related: Vec<(usize, bool, String)>,
}

/// A relationship instance that omits an end its schema requires.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AbsentEnd {
    pub(crate) instance: EntityId,
    pub(crate) type_name: String,
    pub(crate) attribute: String,
}

/// All edges of one requested relationship type, read once per session.
#[derive(Debug, Default)]
pub(crate) struct EdgeIndex {
    /// relating -> [(related, relationship instance)]
    forward: BTreeMap<EntityId, Vec<(EntityId, EntityId)>>,
    /// related -> [(relating, relationship instance)]
    backward: BTreeMap<EntityId, Vec<(EntityId, EntityId)>>,
    /// Canonical schema name of the requested type.
    type_name: String,
    /// Number of instances scanned, recorded in completeness evidence.
    instances: usize,
    /// Instances that contributed no edge through an absent required end.
    pub(crate) absent: Vec<AbsentEnd>,
}

pub(crate) struct IfcRelationshipService {
    model: Arc<Model>,
    snapshots: Arc<[SourceSnapshot]>,
    cache: Mutex<BTreeMap<String, Result<Arc<EdgeIndex>, RelationshipSelectionError>>>,
}

impl IfcRelationshipService {
    pub(crate) fn new(model: Arc<Model>, snapshots: Arc<[SourceSnapshot]>) -> Self {
        Self {
            model,
            snapshots,
            cache: Mutex::new(BTreeMap::new()),
        }
    }

    fn locator(&self, detail: impl std::fmt::Display) -> String {
        format!("ifc:{}:{detail}", self.snapshots[0].fingerprint())
    }

    fn index(&self, relationship: &str) -> Result<Arc<EdgeIndex>, RelationshipSelectionError> {
        let key = relationship.to_ascii_uppercase();
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| unavailable("relationship index lock is poisoned"))?;
        cache
            .entry(key)
            .or_insert_with(|| build_index(&self.model, relationship).map(Arc::new))
            .clone()
    }

    fn entity(&self, object: &ObjectId) -> Result<EntityId, RelationshipSelectionError> {
        if object.source != *self.snapshots[0].source() {
            return Err(unavailable(format!(
                "object `{object}` is not from this IFC source; relationships across sources are unknown here"
            )));
        }
        let id = object
            .local_id
            .strip_prefix('#')
            .and_then(|digits| digits.parse::<u64>().ok())
            .map(EntityId)
            .ok_or(RelationshipSelectionError::InvalidRequest)?;
        if self.model.get(id).is_none() {
            return Err(RelationshipSelectionError::InvalidRequest);
        }
        Ok(id)
    }
}

impl RelationshipSelectionService for IfcRelationshipService {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn select(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        let anchor = self.entity(request.anchor())?;
        let universe = request
            .candidate_universe()
            .iter()
            .map(|candidate| Ok((self.entity(candidate)?, candidate.clone())))
            .collect::<Result<BTreeMap<_, _>, RelationshipSelectionError>>()?;
        let (relationship, reached, used) = match request.query() {
            RelationshipQuery::Related {
                relationship,
                direction,
                follow_chain,
            } => {
                let index = self.index(relationship.as_str())?;
                let (reached, used) = traverse(&index, anchor, *direction, *follow_chain);
                (index, reached, used)
            }
            RelationshipQuery::SharedGroup { relationship } => {
                let index = self.index(relationship.as_str())?;
                let (reached, used) = shared_group(&index, anchor);
                (index, reached, used)
            }
        };
        if !relationship.absent.is_empty() && request.absent_ends() == AbsentEndPolicy::Refuse {
            let first = &relationship.absent[0];
            return Err(unavailable(format!(
                "{} instance(s) of `{}` omit a required end (first: {} {} has no `{}`); the \
                 missing edges could touch any object, so no answer is complete. Opt in to \
                 skipping them to answer from the edges that exist",
                relationship.absent.len(),
                relationship.type_name,
                first.type_name,
                first.instance,
                first.attribute,
            )));
        }
        let candidates = reached
            .iter()
            .filter(|entity| **entity != anchor)
            .filter_map(|entity| universe.get(entity).cloned())
            .collect();
        let source = self.snapshots[0].source().clone();
        // The scan locator is the completeness proof: every instance of the
        // type was read and none was malformed. It is present even when
        // nothing was selected, which is what makes "none related" exact.
        let mut evidence = vec![Evidence::exact(
            source.clone(),
            self.locator(format_args!(
                "relationship-scan:{}:{}",
                relationship.type_name, relationship.instances
            )),
        )];
        evidence.extend(used.into_iter().map(|instance| {
            Evidence::exact(
                source.clone(),
                self.locator(format_args!("relationship:{instance}")),
            )
        }));
        // Under `Skip` every skipped instance is cited, not just those near
        // the anchor: an absent end is unknown, so it could have been any
        // object, including the anchor itself.
        evidence.extend(relationship.absent.iter().map(|absent| {
            Evidence::exact(
                source.clone(),
                self.locator(format_args!(
                    "relationship-absent-end:{}:{}",
                    absent.instance, absent.attribute
                )),
            )
        }));
        CompleteRelationshipSelection::try_new(request.clone(), candidates, evidence)
    }
}

type Reached = (BTreeSet<EntityId>, BTreeSet<EntityId>);

fn traverse(
    index: &EdgeIndex,
    anchor: EntityId,
    direction: TraversalDirection,
    follow_chain: bool,
) -> Reached {
    let mut reached = BTreeSet::new();
    let mut used = BTreeSet::new();
    let mut seen = BTreeSet::from([anchor]);
    let mut queue = VecDeque::from([anchor]);
    while let Some(current) = queue.pop_front() {
        let forward = matches!(
            direction,
            TraversalDirection::Forward | TraversalDirection::Either
        );
        let backward = matches!(
            direction,
            TraversalDirection::Backward | TraversalDirection::Either
        );
        let edges = forward
            .then(|| index.forward.get(&current))
            .flatten()
            .into_iter()
            .chain(backward.then(|| index.backward.get(&current)).flatten())
            .flatten();
        for (next, instance) in edges {
            used.insert(*instance);
            reached.insert(*next);
            if follow_chain && seen.insert(*next) {
                queue.push_back(*next);
            }
        }
    }
    (reached, used)
}

/// Members of any group the anchor belongs to.
///
/// A group is the relating end of an instance; the anchor is a member when it
/// is one of the related ends, and so is every other related end of the same
/// relating entity (across instances, since a file may split one storey's
/// containment over several relationship entities).
fn shared_group(index: &EdgeIndex, anchor: EntityId) -> Reached {
    let mut reached = BTreeSet::new();
    let mut used = BTreeSet::new();
    for (group, instance) in index.backward.get(&anchor).into_iter().flatten() {
        used.insert(*instance);
        for (member, member_instance) in index.forward.get(group).into_iter().flatten() {
            used.insert(*member_instance);
            reached.insert(*member);
        }
    }
    (reached, used)
}

fn build_index(model: &Model, relationship: &str) -> Result<EdgeIndex, RelationshipSelectionError> {
    let schema = ifc4();
    let requested = schema
        .entity(relationship)
        .filter(|entity| schema.is_a(&entity.name, "IfcRelationship"))
        .ok_or_else(|| {
            unavailable(format!(
                "`{relationship}` is not an IFC4 relationship entity type"
            ))
        })?;
    let mut index = EdgeIndex {
        type_name: requested.name.clone(),
        ..EdgeIndex::default()
    };
    // The requested type and every subtype, walked through the model's type
    // index: cost follows the number of relationships, not the model size.
    let mut types = vec![requested.name.as_str()];
    types.extend(schema.subtypes(&requested.name));
    types.sort_unstable();
    types.dedup();
    for type_name in types {
        // The shape is checked for every instantiable type, not only those
        // present: whether a request is answerable is a property of the
        // relationship type, and must not flip with the file's contents.
        if schema
            .entity(type_name)
            .is_some_and(|entity| entity.abstract_)
        {
            continue;
        }
        let ends = ends_of(schema, type_name)?;
        for id in model.ids_of_type(type_name) {
            read_instance(model, *id, &ends, &mut index)?;
        }
    }
    Ok(index)
}

pub(crate) fn read_instance(
    model: &Model,
    id: EntityId,
    ends: &Ends,
    index: &mut EdgeIndex,
) -> Result<(), RelationshipSelectionError> {
    let entity = model
        .get(id)
        .ok_or_else(|| malformed(id, "is indexed but absent from the model"))?;
    index.instances += 1;
    let mut absent = |attribute: &str| {
        index.absent.push(AbsentEnd {
            instance: id,
            // The schema's canonical spelling, not the file's upper case, so
            // the same instance reads the same in every report.
            type_name: ifc4()
                .entity(&entity.type_name)
                .map_or_else(|| entity.type_name.to_string(), |def| def.name.clone()),
            attribute: attribute.to_owned(),
        });
    };
    let relating_value = entity.attribute(ends.relating);
    if is_absent(relating_value) {
        if !ends.relating_optional {
            absent(&ends.relating_name);
        }
        return Ok(());
    }
    let relating = single_end(model, id, relating_value)?;
    let mut related_ends = Vec::new();
    for (slot, optional, name) in &ends.related {
        let value = entity.attribute(*slot);
        if is_absent(value) {
            if !optional {
                absent(name);
            }
            continue;
        }
        related_ends.extend(end_list(model, id, value)?);
    }
    for related in related_ends {
        index
            .forward
            .entry(relating)
            .or_default()
            .push((related, id));
        index
            .backward
            .entry(related)
            .or_default()
            .push((relating, id));
    }
    Ok(())
}

/// `$` or a missing trailing slot: the file states no value for the end.
fn is_absent(value: Option<&Value>) -> bool {
    matches!(value, None | Some(Value::Null))
}

/// Slots of the relating and related ends, from the normative schema.
pub(crate) fn ends_of(
    schema: &Schema,
    type_name: &str,
) -> Result<Ends, RelationshipSelectionError> {
    let attributes = schema.attributes(type_name);
    let mut mixed = None;
    let mut object_end = |prefix: &str| -> Vec<(usize, bool, String)> {
        let mut ends = Vec::new();
        for (slot, attribute) in attributes.iter().enumerate() {
            if !attribute.name.starts_with(prefix) {
                continue;
            }
            match reference_kind(schema, &attribute.type_name) {
                References::Entities => {
                    ends.push((slot, attribute.optional, attribute.name.clone()));
                }
                // Enumerations and measures such as `RelatedPriorities`
                // qualify an edge; they are not ends.
                References::None => {}
                References::Mixed => mixed = Some(attribute.name.clone()),
            }
        }
        ends
    };
    let relating = object_end("Relating");
    let related = object_end("Related");
    if let Some(attribute) = mixed {
        // An end that can hold either a reference or a non-reference value
        // would lose edges whenever it holds the latter.
        return Err(unavailable(format!(
            "`{type_name}.{attribute}` mixes entity references with other values; its edges cannot be read exactly"
        )));
    }
    match (relating.as_slice(), related.is_empty()) {
        ([(slot, optional, name)], false) => Ok(Ends {
            relating: *slot,
            relating_optional: *optional,
            relating_name: name.clone(),
            related,
        }),
        _ => Err(unavailable(format!(
            "`{type_name}` has no single object-to-object relating end; its edges cannot be read exactly"
        ))),
    }
}

/// Whether an attribute type can only hold entity references.
/// What an attribute type can hold, as far as entity references go.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum References {
    /// Only entity references.
    Entities,
    /// No entity references at all.
    None,
    /// Entity references or other values.
    Mixed,
}

fn reference_kind(schema: &Schema, type_name: &str) -> References {
    if schema.entity(type_name).is_some() {
        return References::Entities;
    }
    match schema
        .type_def(type_name)
        .map(|definition| &definition.kind)
    {
        Some(TypeKind::Select(items)) => {
            let kinds: BTreeSet<_> = items
                .iter()
                .map(|item| reference_kind(schema, item))
                .collect();
            match kinds.iter().collect::<Vec<_>>().as_slice() {
                [References::Entities] => References::Entities,
                [References::None] | [] => References::None,
                _ => References::Mixed,
            }
        }
        // A defined type keeps its right-hand side as text. One that wraps
        // entities, e.g. `IfcPropertySetDefinitionSet` =
        // `SET [1:?] OF IfcPropertySetDefinition`, nests an aggregate inside
        // the end, which the edge reader does not flatten, so it is refused.
        Some(TypeKind::Defined(text))
            if text
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .any(|word| schema.entity(word).is_some()) =>
        {
            References::Mixed
        }
        _ => References::None,
    }
}

fn single_end(
    model: &Model,
    instance: EntityId,
    value: Option<&Value>,
) -> Result<EntityId, RelationshipSelectionError> {
    match value {
        Some(Value::Ref(target)) => resolved(model, instance, *target),
        _ => Err(malformed(
            instance,
            "relating end is not one entity reference",
        )),
    }
}

fn end_list(
    model: &Model,
    instance: EntityId,
    value: Option<&Value>,
) -> Result<Vec<EntityId>, RelationshipSelectionError> {
    match value {
        Some(Value::Ref(target)) => Ok(vec![resolved(model, instance, *target)?]),
        Some(Value::List(items)) => items
            .iter()
            .map(|item| match item {
                Value::Ref(target) => resolved(model, instance, *target),
                _ => Err(malformed(instance, "related end holds a non-reference")),
            })
            .collect(),
        _ => Err(malformed(
            instance,
            "related end is not an entity reference",
        )),
    }
}

fn resolved(
    model: &Model,
    instance: EntityId,
    target: EntityId,
) -> Result<EntityId, RelationshipSelectionError> {
    if model.get(target).is_some() {
        Ok(target)
    } else {
        Err(malformed(
            instance,
            &format!("references missing entity {target}"),
        ))
    }
}

fn malformed(instance: EntityId, detail: &str) -> RelationshipSelectionError {
    unavailable(format!("relationship {instance} is malformed: {detail}"))
}

fn unavailable(message: impl Into<String>) -> RelationshipSelectionError {
    RelationshipSelectionError::Unavailable(message.into())
}
