//! Exact relationship selection over the objectified relationships of one IFC model.
//!
//! IFC stores no parent pointers: an `IfcRelContainedInSpatialStructure`
//! entity names both the storey and its elements. A relationship identity in a
//! request is therefore the name of a relationship entity type in the
//! session's release (IFC2X3 or IFC4), and every
//! instance of that type (or of a subtype) is one edge set from its relating
//! end to each of its related ends.
//!
//! Which attribute slot is which end, and whether it is optional, is read from
//! the bundled normative schema of the file's own release rather than
//! hard-coded (`IfcRelSpaceBoundary.RelatedBuildingElement` is optional in
//! IFC2X3, required in IFC4): the slot positions differ per type
//! (`IfcRelAggregates` keeps the relating end in slot 4, containment in slot
//! 5, `IfcRelConnectsElements` in slot 5 because connection geometry takes 4),
//! and a hand-written table silently inverts direction when one is wrong.
//!
//! Every answer is complete for its source or refused. A malformed instance, a
//! reference to a missing entity, a relationship type whose ends are not plain
//! object references, or a candidate from another source makes the whole
//! answer `Unavailable`: leaving one edge out would turn "not related" into a
//! claim the file does not make.

use std::any::TypeId;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use axioval_engine::{
    AbsentEndPolicy, CandidateUniverse, CompleteRelationshipEdges, CompleteRelationshipSelection,
    DerivationError, DerivationKind, DerivationStore, DerivationVersion, Inputs, Precision,
    RELATIONSHIP_KIND_PREFIX, RelationshipEdge, RelationshipEdgesRequest, RelationshipKind,
    RelationshipQuery, RelationshipSelectionError, RelationshipSelectionRequest,
    RelationshipSelectionService, RelationshipSelectionServiceHandle, SemanticRelationship,
    SourceSnapshot, TraversalDirection,
};
use axioval_ir::{Evidence, ObjectId};
use ifc_model::{EntityId, Model, Value};
use ifc_schema::{Schema, TypeKind};

use crate::release::Release;

/// End slots of one concrete relationship entity type.
#[derive(Clone, Debug)]
pub(crate) struct Ends {
    /// Canonical schema spelling of the concrete type, used in reports.
    type_name: String,
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
    pub(crate) forward: BTreeMap<EntityId, Vec<(EntityId, EntityId)>>,
    /// related -> [(relating, relationship instance)]
    pub(crate) backward: BTreeMap<EntityId, Vec<(EntityId, EntityId)>>,
    /// Canonical schema name of the requested type.
    pub(crate) type_name: String,
    /// Number of instances scanned, recorded in completeness evidence.
    pub(crate) instances: usize,
    /// Instances that contributed no edge through an absent required end.
    pub(crate) absent: Vec<AbsentEnd>,
}

/// A candidate universe by entity, or why it is refused.
type Entities = Result<Arc<BTreeMap<EntityId, ObjectId>>, RelationshipSelectionError>;

pub(crate) struct IfcRelationshipService {
    release: Release,
    model: Arc<Model>,
    snapshots: Arc<[SourceSnapshot]>,
    /// Each relationship type's edges, indexed once: an index of the model
    /// (#443), the same for every check of it, so kept with the source
    /// rather than in a check's derivation store.
    cache: Mutex<BTreeMap<String, Result<Arc<EdgeIndex>, RelationshipSelectionError>>>,
}

/// The model of a universe read by entity, compared and hashed by its
/// address.
#[derive(Clone)]
pub(crate) struct OverModel(Arc<Model>);

impl PartialEq for OverModel {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for OverModel {}

impl Hash for OverModel {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

/// Everything a universe read by entity is read from (#439): the model (by
/// identity), the source its objects must belong to, and the universe's
/// objects (a shared universe compared by its identity first, any other by
/// its objects, hashed once when it was built); the kind's version beside
/// it. Requests bound to one universe, or to equal universes built apart
/// (a rule relating each anchor to the objects it selected), read one
/// entry.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct UniverseKey {
    model: OverModel,
    source: axioval_ir::SourceId,
    universe: CandidateUniverse,
}

/// The derivation kind of candidate universes read by entity (#443): a
/// rule asks about many anchors over one universe, each request bound to
/// it, so its candidates are read by entity once per check rather than
/// per request, and released after the last rule that may select a
/// relationship.
pub(crate) struct UniverseEntities;

impl DerivationKind for UniverseEntities {
    const NAME: &'static str = "universe entities";
    /// Bumped whenever the same universe could be read otherwise: the
    /// reading of an object's instance id changing.
    const VERSION: DerivationVersion = DerivationVersion::new(1, &[]);
    type Key = UniverseKey;
    type Value = Entities;

    fn build(key: &UniverseKey, _: &Inputs<'_>) -> Result<Self::Value, DerivationError> {
        Ok(key
            .universe
            .objects()
            .iter()
            .map(|candidate| {
                Ok((
                    entity_in(&key.model.0, &key.source, candidate)?,
                    candidate.clone(),
                ))
            })
            .collect::<Result<BTreeMap<_, _>, RelationshipSelectionError>>()
            .map(Arc::new))
    }

    /// Not geometric: an instance id is read exactly or refused.
    fn precision(_: &UniverseKey) -> Option<Precision> {
        None
    }

    fn size_hint(value: &Self::Value) -> Option<usize> {
        Some(value.as_ref().map_or(0, |entities| {
            entities
                .values()
                .map(|object| {
                    object.local_id.len()
                        + std::mem::size_of::<(EntityId, ObjectId)>()
                        + 2 * std::mem::size_of::<usize>()
                })
                .sum()
        }))
    }

    /// At most a few universes' worth, as the service's ring of four kept
    /// before: a rule relates its anchors to one or two, and one built per
    /// request is never read again.
    fn keep_bytes() -> Option<u64> {
        Some(UNIVERSES_KEPT)
    }

    /// Universes are read by entity for the relationship service alone.
    fn read_through() -> Option<Vec<TypeId>> {
        Some(vec![TypeId::of::<RelationshipSelectionServiceHandle>()])
    }
}

/// The most bytes of universes read by entity the store keeps at once.
const UNIVERSES_KEPT: u64 = 64 << 20;

/// `object`'s instance in `model`, as one of `source`'s.
fn entity_in(
    model: &Model,
    source: &axioval_ir::SourceId,
    object: &ObjectId,
) -> Result<EntityId, RelationshipSelectionError> {
    if object.source != *source {
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
    if model.get(id).is_none() {
        return Err(RelationshipSelectionError::InvalidRequest);
    }
    Ok(id)
}

impl IfcRelationshipService {
    pub(crate) fn new(
        release: Release,
        model: Arc<Model>,
        snapshots: Arc<[SourceSnapshot]>,
    ) -> Self {
        Self {
            release,
            model,
            snapshots,
            cache: Mutex::new(BTreeMap::new()),
        }
    }

    /// The candidates of `universe` by entity, each refused as `entity`
    /// refuses it: the check's, read once per shared universe for every
    /// request bound to it ([`UniverseEntities`]).
    fn entities(&self, universe: &CandidateUniverse) -> Entities {
        let key = UniverseKey {
            model: OverModel(Arc::clone(&self.model)),
            source: self.snapshots[0].source().clone(),
            universe: universe.clone(),
        };
        DerivationStore::derive_current::<UniverseEntities>(&key)
            .map_err(|error| unavailable(error.to_string()))?
            .as_ref()
            .clone()
    }

    fn locator(&self, detail: impl std::fmt::Display) -> String {
        format!("ifc:{}:{detail}", self.snapshots[0].fingerprint())
    }

    fn index(&self, relationship: &str) -> Result<Arc<EdgeIndex>, RelationshipSelectionError> {
        let relationship = entity_of(relationship)?;
        let key = relationship.to_ascii_uppercase();
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| unavailable("relationship index lock is poisoned"))?;
        cache
            .entry(key)
            .or_insert_with(|| build_index(self.release, &self.model, relationship).map(Arc::new))
            .clone()
    }

    fn entity(&self, object: &ObjectId) -> Result<EntityId, RelationshipSelectionError> {
        entity_in(&self.model, self.snapshots[0].source(), object)
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
        let universe = self.entities(request.universe())?;
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
        refuse_absent_ends(&relationship, request.absent_ends())?;
        let candidates = reached
            .iter()
            .filter(|entity| **entity != anchor)
            .filter_map(|entity| universe.get(entity).cloned())
            .collect();
        let evidence = self.evidence(&relationship, used);
        CompleteRelationshipSelection::try_new(request.clone(), candidates, evidence)
    }

    /// A chain is a breadth-first closure over the type's index, the
    /// universe only filtering what it answers: a chain from any object it
    /// reaches reaches nothing more and reads no edge it did not.
    fn closes_chains(&self, _relationship: &SemanticRelationship) -> bool {
        true
    }

    /// Every edge of the type whose two ends lie in the universe, read from
    /// the same index and refused on the same grounds as a selection.
    fn edges(
        &self,
        request: &RelationshipEdgesRequest,
    ) -> Result<CompleteRelationshipEdges, RelationshipSelectionError> {
        let universe = request
            .universe()
            .iter()
            .map(|object| Ok((self.entity(object)?, object.clone())))
            .collect::<Result<BTreeMap<_, _>, RelationshipSelectionError>>()?;
        let relationship = self.index(request.relationship().as_str())?;
        refuse_absent_ends(&relationship, request.absent_ends())?;
        let mut edges = BTreeSet::new();
        let mut used = BTreeSet::new();
        for (relating, related) in &relationship.forward {
            let Some(relating) = universe.get(relating) else {
                continue;
            };
            for (related, instance) in related {
                if let Some(related) = universe.get(related) {
                    used.insert(*instance);
                    edges.insert(RelationshipEdge {
                        relating: relating.clone(),
                        related: related.clone(),
                    });
                }
            }
        }
        let evidence = self.evidence(&relationship, used);
        CompleteRelationshipEdges::try_new(request.clone(), edges.into_iter().collect(), evidence)
    }
}

impl IfcRelationshipService {
    /// The completeness proof of an answer from `relationship`, the
    /// instances it used, and every instance skipped for an absent end.
    fn evidence(&self, relationship: &EdgeIndex, used: BTreeSet<EntityId>) -> Vec<Evidence> {
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
        evidence
    }
}

/// Refuses an answer from a type with instances lacking a required end,
/// unless the request opts into skipping them.
fn refuse_absent_ends(
    relationship: &EdgeIndex,
    policy: AbsentEndPolicy,
) -> Result<(), RelationshipSelectionError> {
    let Some(first) = relationship.absent.first() else {
        return Ok(());
    };
    if policy == AbsentEndPolicy::Skip {
        return Ok(());
    }
    Err(unavailable(format!(
        "{} instance(s) of `{}` omit a required end (first: {} {} has no `{}`); the \
         missing edges could touch any object, so no answer is complete. Opt in to \
         skipping them to answer from the edges that exist",
        relationship.absent.len(),
        relationship.type_name,
        first.type_name,
        first.instance,
        first.attribute,
    )))
}

/// The IFC relationship entity a requested identity names: a
/// [`RelationshipKind`] identity maps onto the one objectified relationship
/// type stating that kind in every release, any other identity is an entity
/// name itself.
fn entity_of(relationship: &str) -> Result<&str, RelationshipSelectionError> {
    if !relationship.starts_with(RELATIONSHIP_KIND_PREFIX) {
        return Ok(relationship);
    }
    let kind = RelationshipKind::of(relationship).ok_or_else(|| {
        unavailable(format!(
            "`{relationship}` is not a relationship kind the IFC adapter answers"
        ))
    })?;
    Ok(match kind {
        RelationshipKind::Containment => "IfcRelContainedInSpatialStructure",
        RelationshipKind::Aggregation => "IfcRelAggregates",
        RelationshipKind::Voids => "IfcRelVoidsElement",
        RelationshipKind::Fills => "IfcRelFillsElement",
        RelationshipKind::SpaceBoundary => "IfcRelSpaceBoundary",
        RelationshipKind::TypeAssignment => "IfcRelDefinesByType",
        RelationshipKind::GroupMembership => "IfcRelAssignsToGroup",
        RelationshipKind::Connection => "IfcRelConnectsElements",
        _ => {
            return Err(unavailable(format!(
                "`{relationship}` is not a relationship kind the IFC adapter answers"
            )));
        }
    })
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

pub(crate) fn build_index(
    release: Release,
    model: &Model,
    relationship: &str,
) -> Result<EdgeIndex, RelationshipSelectionError> {
    let schema = release.schema;
    let requested = schema
        .entity(relationship)
        .filter(|entity| schema.is_a(&entity.name, "IfcRelationship"))
        .ok_or_else(|| {
            unavailable(format!(
                "`{relationship}` is not an {} relationship entity type",
                release.label
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
            type_name: ends.type_name.clone(),
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
            type_name: schema
                .entity(type_name)
                .map_or_else(|| type_name.to_owned(), |entity| entity.name.clone()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_engine::KeyCompleteness;
    use axioval_ir::SourceId;

    const STEP: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\n\
        FILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
        #1=IFCWALL('0000000000000000000001',$,$,$,$,$,$,$,$);\n\
        #2=IFCSLAB('0000000000000000000002',$,$,$,$,$,$,$,$);\n\
        ENDSEC;\nEND-ISO-10303-21;\n";

    fn model() -> Arc<Model> {
        Arc::new(crate::step::read_ifc_step(STEP.as_bytes()).unwrap())
    }

    fn object(source: &SourceId, local: &str) -> ObjectId {
        ObjectId::new(source.clone(), local).unwrap()
    }

    fn read(value: &Entities) -> String {
        format!("{value:?}")
    }

    /// Every input a universe read by entity is read from is in its key
    /// (#439).
    #[test]
    fn changing_any_input_of_a_universe_read_by_entity_changes_its_key() {
        let (one, other) = (model(), model());
        let source = SourceId::new("ifc", "m").unwrap();
        let elsewhere = SourceId::new("ifc", "n").unwrap();
        let universe = |locals: &[&str]| {
            CandidateUniverse::try_new(locals.iter().map(|local| object(&source, local)).collect())
                .unwrap()
        };
        let both = universe(&["#1", "#2"]);
        let key =
            |model: &Arc<Model>, source: &SourceId, universe: &CandidateUniverse| UniverseKey {
                model: OverModel(Arc::clone(model)),
                source: source.clone(),
                universe: universe.clone(),
            };
        KeyCompleteness::<UniverseEntities, _>::new(key(&one, &source, &both), read)
            .same("the same inputs", key(&one, &source, &both))
            // Another parse of the same file reads alike.
            .input("the model", key(&other, &source, &both))
            .changes_value("the source", key(&one, &elsewhere, &both))
            .changes_value("the universe", key(&one, &source, &universe(&["#1"])))
            // Equal objects built apart: the same entry.
            .same(
                "an equal universe",
                key(&one, &source, &universe(&["#2", "#1"])),
            )
            .check();
    }
}
