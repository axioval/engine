//! Relations a ruleset declares between objects (`relations`).
//!
//! A relation pairs objects the model does not relate: the pumps serving
//! each room, the details belonging to each wall. Its pairs are listed
//! (by object identity or external id) or matched (equal property values
//! on both ends), and the relationship `axioval:derived.relation;id=<id>`
//! runs from each from-object to its to-objects, `backward` the other way,
//! in every relationship path.
//!
//! Nothing undecided is guessed. A from-object whose selection or key
//! cannot be read could relate to any to-object, so every to-object's
//! sources are then undecided, and the other way round. A listed pair
//! naming an object the model does not hold, or one its end's selector
//! does not select, leaves the other end's partners undecided (both ends'
//! when neither is known), and [`unknown_relation_objects`] lists it.
//!
//! A relation `by: supplied` takes its pairs from the host at check time
//! ([`SuppliedPairs`], [`ExecutionPlan::supply_relation`]), so one rule
//! package serves every project. Without them nothing is related surely:
//! every object's partners are undecided, never none.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axioval_ir::contract::{ParameterValue, RelationDefinition, RelationKey, TableRow};
use axioval_ir::{Evidence, Object, ObjectId, Project};

use crate::derived_relationships::DERIVED_RELATIONSHIP_PREFIX;
use crate::groupings::property_key;
use crate::relationships::{
    CompleteRelationshipEdges, CompleteRelationshipSelection, RelationshipEdgesRequest,
    RelationshipQuery, RelationshipSelectionError, RelationshipSelectionRequest,
    RelationshipSelectionService, RelationshipSelectionServiceHandle, TraversalDirection,
};
use crate::session::SourceSnapshot;
use crate::{ExecutionPlan, OutcomeRefiner, RuleContext, SelectorVerdict, ServiceRegistry};

/// The relationship identity of a declared relation, before its id:
/// `axioval:derived.relation;id=<id>`.
pub const RELATION_RELATIONSHIP_PREFIX: &str = "axioval:derived.relation;id=";

/// The pairs of one declared relation, as far as they are known.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Relation {
    forward: BTreeMap<ObjectId, BTreeSet<ObjectId>>,
    backward: BTreeMap<ObjectId, BTreeSet<ObjectId>>,
    /// The facts deciding each pair, by its from- and to-object.
    evidence: BTreeMap<(ObjectId, ObjectId), Vec<Evidence>>,
    /// From-objects whose to-objects are not known completely, and why.
    open_forward: BTreeMap<ObjectId, String>,
    /// To-objects whose from-objects are not known completely, and why.
    open_backward: BTreeMap<ObjectId, String>,
    /// Why no from-object's to-objects are known completely, if so.
    all_forward: Option<String>,
    /// Why no to-object's from-objects are known completely, if so.
    all_backward: Option<String>,
}

impl Relation {
    /// The to-objects `from` relates to, or why they are not known.
    ///
    /// # Errors
    ///
    /// Why some pair `from` may be part of is undecided.
    pub fn targets(&self, from: &ObjectId) -> Result<Vec<ObjectId>, &str> {
        if let Some(why) = self
            .all_forward
            .as_deref()
            .or(self.open_forward.get(from).map(String::as_str))
        {
            return Err(why);
        }
        Ok(self
            .forward
            .get(from)
            .map(|to| to.iter().cloned().collect())
            .unwrap_or_default())
    }

    /// The from-objects relating to `to`, or why they are not known.
    ///
    /// # Errors
    ///
    /// Why some pair `to` may be part of is undecided.
    pub fn sources(&self, to: &ObjectId) -> Result<Vec<ObjectId>, &str> {
        if let Some(why) = self
            .all_backward
            .as_deref()
            .or(self.open_backward.get(to).map(String::as_str))
        {
            return Err(why);
        }
        Ok(self
            .backward
            .get(to)
            .map(|from| from.iter().cloned().collect())
            .unwrap_or_default())
    }

    fn relate(&mut self, from: &ObjectId, to: &ObjectId, evidence: Vec<Evidence>) {
        if from == to {
            return;
        }
        self.forward
            .entry(from.clone())
            .or_default()
            .insert(to.clone());
        self.backward
            .entry(to.clone())
            .or_default()
            .insert(from.clone());
        self.evidence
            .entry((from.clone(), to.clone()))
            .or_default()
            .extend(evidence);
    }

    fn open_from(&mut self, from: &ObjectId, why: &str) {
        self.open_forward
            .entry(from.clone())
            .or_insert_with(|| why.to_owned());
    }

    fn open_to(&mut self, to: &ObjectId, why: &str) {
        self.open_backward
            .entry(to.clone())
            .or_insert_with(|| why.to_owned());
    }
}

/// Every relation a run derives, by id.
#[derive(Clone, Debug, Default)]
pub struct DeclaredRelations {
    relations: BTreeMap<String, Relation>,
}

impl DeclaredRelations {
    /// Adds the relation `id`.
    #[must_use]
    pub fn with_relation(mut self, id: impl Into<String>, relation: Relation) -> Self {
        self.relations.insert(id.into(), relation);
        self
    }

    /// The relation `id`, if the run derives it.
    #[must_use]
    pub fn relation(&self, id: &str) -> Option<&Relation> {
        self.relations.get(id)
    }
}

/// The pairs of a supplied relation (`by: supplied`), as the host read
/// them at check time: their origin, the SHA-256 of the file they were
/// read from, and one row of non-blank text `from` and `to` cells each.
#[derive(Clone, Debug, PartialEq)]
pub struct SuppliedPairs {
    origin: String,
    sha256: String,
    rows: Vec<TableRow>,
}

impl SuppliedPairs {
    /// Pairs the host already read: `origin` names where they came from
    /// (such as a file name), `sha256` is the lowercase hex SHA-256 of the
    /// bytes they were read from, recorded in every pair's evidence.
    #[must_use]
    pub fn new(origin: impl Into<String>, sha256: impl Into<String>, rows: Vec<TableRow>) -> Self {
        Self {
            origin: origin.into(),
            sha256: sha256.into(),
            rows,
        }
    }

    /// Reads the pairs of the supplied relation `definition` from `bytes`,
    /// the file `name` (a `.csv` file, or the sheet `sheet` of an `.xlsx`
    /// workbook), by the columns it declares, exactly as a table file is
    /// read ([`read_table_file`](crate::read_table_file)).
    ///
    /// # Errors
    ///
    /// The relation is not supplied, or why the file or its columns were
    /// refused.
    pub fn read(
        definition: &RelationDefinition,
        bytes: &[u8],
        name: &str,
        sheet: Option<&str>,
    ) -> Result<Self, String> {
        let RelationKey::Supplied { columns, .. } = &definition.by else {
            return Err(format!(
                "relation `{}` states its own pairs; none are supplied",
                definition.id
            ));
        };
        let columns = crate::compiler::supplied_columns(columns.as_deref())?;
        let rows = crate::table_files::read_table_file(bytes, name, sheet, &columns)?;
        let origin = match sheet {
            Some(sheet) => format!("{name}#{sheet}"),
            None => name.to_owned(),
        };
        Ok(Self::new(
            origin,
            crate::table_files::sha256_hex(bytes),
            rows,
        ))
    }

    /// Where the pairs came from.
    #[must_use]
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// The SHA-256 of the bytes the pairs were read from.
    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// The pairs, one row each.
    #[must_use]
    pub fn rows(&self) -> &[TableRow] {
        &self.rows
    }
}

/// One listed pair's end naming no single object of the project.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnknownRelationObject {
    /// The relation's id.
    pub relation: String,
    /// The pair's one-based row.
    pub row: usize,
    /// `from` or `to`.
    pub end: &'static str,
    /// The identity as listed.
    pub identity: String,
    /// Why it names no single object.
    pub detail: String,
}

/// Every end of a listed or supplied pair of `plan`'s relations that names
/// no object of `project`, or several, in relation and row order.
#[must_use]
pub fn unknown_relation_objects(
    plan: &ExecutionPlan,
    project: &Project,
) -> Vec<UnknownRelationObject> {
    let mut unknown = Vec::new();
    for definition in plan.relations() {
        let (rows, scheme) = match &definition.by {
            RelationKey::Pairs { pairs, scheme } => (table_rows(pairs), scheme),
            RelationKey::Supplied { scheme, .. } => match plan.supplied_pairs(&definition.id) {
                Some(supplied) => (supplied.rows(), scheme),
                None => continue,
            },
            RelationKey::Property { .. } => continue,
        };
        let index = identities(project, scheme.as_deref());
        for (row, from, to) in listed(rows) {
            for (end, identity) in [("from", from), ("to", to)] {
                if let Err(detail) = resolve(&index, &identity, scheme.as_deref()) {
                    unknown.push(UnknownRelationObject {
                        relation: definition.id.clone(),
                        row,
                        end,
                        identity,
                        detail,
                    });
                }
            }
        }
    }
    unknown
}

/// The rows of a compiled `pairs` table.
fn table_rows(pairs: &ParameterValue) -> &[TableRow] {
    match pairs {
        ParameterValue::Table { value: rows } => rows,
        _ => &[],
    }
}

/// The pairs of `rows`: one-based row, from, to.
fn listed(rows: &[TableRow]) -> Vec<(usize, String, String)> {
    rows.iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let text = |column: &str| match row.get(column) {
                Some(ParameterValue::String { value }) => Some(value.clone()),
                _ => None,
            };
            Some((index + 1, text("from")?, text("to")?))
        })
        .collect()
}

/// Every object of `project` by the identity a pair names it by: its
/// identity as reports write it, or its external id in `scheme`.
fn identities(project: &Project, scheme: Option<&str>) -> BTreeMap<String, Vec<ObjectId>> {
    let mut index: BTreeMap<String, Vec<ObjectId>> = BTreeMap::new();
    for object in project.objects() {
        let key = match scheme {
            None => Some(object.id.to_string()),
            Some(scheme) => object.external_id(scheme).map(str::to_owned),
        };
        if let Some(key) = key {
            index.entry(key).or_default().push(object.id.clone());
        }
    }
    index
}

fn resolve(
    index: &BTreeMap<String, Vec<ObjectId>>,
    identity: &str,
    scheme: Option<&str>,
) -> Result<ObjectId, String> {
    match index.get(identity).map(Vec::as_slice) {
        Some([object]) => Ok(object.clone()),
        Some(several) if several.len() > 1 => Err(format!(
            "`{identity}` names {} objects in `{}`",
            several.len(),
            scheme.unwrap_or("identity")
        )),
        _ => Err(match scheme {
            Some(scheme) => format!("no object of the model carries `{identity}` in `{scheme}`"),
            None => format!("`{identity}` is no object of the model"),
        }),
    }
}

/// Derives the relation `definition` over every object of the context's
/// project, a supplied relation from `supplied`.
pub(crate) fn derive(
    refiner: &dyn OutcomeRefiner,
    context: &RuleContext<'_>,
    definition: &RelationDefinition,
    supplied: Option<&SuppliedPairs>,
) -> Relation {
    match &definition.by {
        RelationKey::Property { from, to } => {
            let mut relation = Relation::default();
            let mut from_keys: Vec<(ObjectId, String, Vec<Evidence>)> = Vec::new();
            let mut to_keys: BTreeMap<String, Vec<(ObjectId, Vec<Evidence>)>> = BTreeMap::new();
            for object in context.project.objects() {
                let end =
                    |selector, property: &axioval_ir::contract::RelationProperty| match refiner
                        .evaluate_selector(context, selector, object)
                    {
                        SelectorVerdict::NoMatch(_) => None,
                        SelectorVerdict::Undecided(_, why) => Some(Err(why)),
                        SelectorVerdict::Match(_) => Some(
                            property_key(
                                refiner,
                                context,
                                object,
                                property.property_set.as_deref(),
                                &property.property,
                            )
                            .map_err(|(_, why)| why),
                        ),
                    };
                match end(&definition.from, from) {
                    None | Some(Ok(None)) => {}
                    Some(Ok(Some((key, evidence)))) => {
                        from_keys.push((object.id.clone(), key, evidence));
                    }
                    Some(Err(why)) => {
                        let why = format!(
                            "whether {} relates in `{}` is undecided: {why}",
                            object.id, definition.id
                        );
                        relation.open_from(&object.id, &why);
                        relation.all_backward.get_or_insert(why);
                    }
                }
                match end(&definition.to, to) {
                    None | Some(Ok(None)) => {}
                    Some(Ok(Some((key, evidence)))) => {
                        to_keys
                            .entry(key)
                            .or_default()
                            .push((object.id.clone(), evidence));
                    }
                    Some(Err(why)) => {
                        let why = format!(
                            "whether {} is related in `{}` is undecided: {why}",
                            object.id, definition.id
                        );
                        relation.open_to(&object.id, &why);
                        relation.all_forward.get_or_insert(why);
                    }
                }
            }
            for (from, key, evidence) in from_keys {
                for (to, cited) in to_keys.get(&key).into_iter().flatten() {
                    let mut facts = evidence.clone();
                    facts.extend(cited.iter().cloned());
                    facts.push(Evidence::exact(
                        from.source.clone(),
                        format!(
                            "{RELATION_RELATIONSHIP_PREFIX}{}:{from}->{to}#key={key}",
                            definition.id
                        ),
                    ));
                    relation.relate(&from, to, facts);
                }
            }
            relation
        }
        RelationKey::Pairs { pairs, scheme } => derive_pairs(
            refiner,
            context,
            definition,
            (table_rows(pairs), "pairs"),
            scheme.as_deref(),
        ),
        RelationKey::Supplied { scheme, .. } => {
            if let Some(supplied) = supplied {
                derive_pairs(
                    refiner,
                    context,
                    definition,
                    (
                        supplied.rows(),
                        &format!("pairs;sha256={}", supplied.sha256()),
                    ),
                    scheme.as_deref(),
                )
            } else {
                unsupplied(definition)
            }
        }
    }
}

/// A supplied relation given no pairs: no pair is known, so none is ruled
/// out, and every walk is undecided.
fn unsupplied(definition: &RelationDefinition) -> Relation {
    let why = format!(
        "relation `{}` takes its pairs from the host at check time, and none were supplied",
        definition.id
    );
    Relation {
        all_forward: Some(why.clone()),
        all_backward: Some(why),
        ..Relation::default()
    }
}

/// Derives a relation from its listed or supplied `rows`, citing each
/// pair at `<identity>:<stem>#row=<row>`.
fn derive_pairs(
    refiner: &dyn OutcomeRefiner,
    context: &RuleContext<'_>,
    definition: &RelationDefinition,
    (rows, stem): (&[TableRow], &str),
    scheme: Option<&str>,
) -> Relation {
    let mut relation = Relation::default();
    let index = identities(context.project, scheme);
    let id = &definition.id;
    for (row, from, to) in listed(rows) {
        // Each end: the object it names, if one, and why it fails, if so.
        let end = |identity: &str, selector, role: &str| -> (Option<ObjectId>, Option<String>) {
            match resolve(&index, identity, scheme) {
                Err(why) => (None, Some(why)),
                Ok(object_id) => {
                    let verdict = context.project.object(&object_id).map(|object: &Object| {
                        refiner.evaluate_selector(context, selector, object)
                    });
                    let why = match verdict {
                        Some(SelectorVerdict::Match(_)) => None,
                        Some(SelectorVerdict::NoMatch(_)) | None => Some(format!(
                            "{object_id} is not selected as the relation's `{role}`"
                        )),
                        Some(SelectorVerdict::Undecided(_, why)) => Some(format!(
                            "whether {object_id} is the relation's `{role}` is undecided: {why}"
                        )),
                    };
                    (Some(object_id), why)
                }
            }
        };
        let (from_id, from_fails) = end(&from, &definition.from, "from");
        let (to_id, to_fails) = end(&to, &definition.to, "to");
        match (&from_id, from_fails, &to_id, to_fails) {
            (Some(from_id), None, Some(to_id), None) => {
                let evidence = vec![Evidence::exact(
                    from_id.source.clone(),
                    format!(
                        "{RELATION_RELATIONSHIP_PREFIX}{id}:{stem}#row={row}:{from_id}->{to_id}"
                    ),
                )];
                relation.relate(from_id, to_id, evidence);
            }
            (_, from_fails, _, to_fails) => {
                let why = format!(
                    "relation `{id}` row {row} ({from} -> {to}) is undecided: {}",
                    [from_fails, to_fails]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join("; ")
                );
                if let Some(from_id) = &from_id {
                    relation.open_from(from_id, &why);
                }
                if let Some(to_id) = &to_id {
                    relation.open_to(to_id, &why);
                }
                // Neither end is known: the pair may be any.
                if from_id.is_none() && to_id.is_none() {
                    relation.all_forward.get_or_insert(why.clone());
                    relation.all_backward.get_or_insert(why);
                }
            }
        }
    }
    relation
}

/// Derives every relation of a run, in order, over `project`, the
/// supplied ones from `supplied`, by relation id.
pub(crate) fn derive_all(
    refiner: &dyn OutcomeRefiner,
    project: &Project,
    services: &ServiceRegistry,
    (definitions, supplied): (&[RelationDefinition], &BTreeMap<String, SuppliedPairs>),
) -> DeclaredRelations {
    let context = RuleContext { project, services };
    definitions
        .iter()
        .fold(DeclaredRelations::default(), |relations, definition| {
            relations.with_relation(
                definition.id.clone(),
                derive(refiner, &context, definition, supplied.get(&definition.id)),
            )
        })
}

/// Installs the run's declared relations beside the host's relationships.
pub(crate) fn install(services: &mut ServiceRegistry, relations: &Arc<DeclaredRelations>) {
    let inner = services
        .get::<RelationshipSelectionServiceHandle>()
        .cloned();
    services.replace(RelationshipSelectionServiceHandle::new(Arc::new(
        RelationRelationships {
            inner,
            relations: relations.clone(),
        },
    )));
    services.replace(relations.clone());
}

/// Answers `axioval:derived.relation;id=<id>` from the run's relations,
/// every other identity from the host's relationship service.
struct RelationRelationships {
    inner: Option<RelationshipSelectionServiceHandle>,
    relations: Arc<DeclaredRelations>,
}

impl RelationRelationships {
    fn answer(
        &self,
        id: &str,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        let relation = self
            .relations
            .relation(id)
            .ok_or(RelationshipSelectionError::InvalidRequest)?;
        let RelationshipQuery::Related {
            direction,
            follow_chain,
            ..
        } = request.query()
        else {
            return Err(RelationshipSelectionError::InvalidRequest);
        };
        let identity = format!("{RELATION_RELATIONSHIP_PREFIX}{id}");
        let anchor = request.anchor();
        let unavailable = |why: &str| RelationshipSelectionError::Unavailable(why.to_owned());
        let mut evidence = vec![Evidence::exact(
            anchor.source.clone(),
            format!("{identity}:derived-from:{anchor}"),
        )];
        let forward = matches!(
            direction,
            TraversalDirection::Forward | TraversalDirection::Either
        );
        let backward = matches!(
            direction,
            TraversalDirection::Backward | TraversalDirection::Either
        );
        let mut reached = BTreeSet::new();
        let mut frontier = vec![anchor.clone()];
        let mut visited = BTreeSet::from([anchor.clone()]);
        while let Some(object) = frontier.pop() {
            let mut next = Vec::new();
            if forward {
                for to in relation.targets(&object).map_err(unavailable)? {
                    if let Some(facts) = relation.evidence.get(&(object.clone(), to.clone())) {
                        evidence.extend(facts.iter().cloned());
                    }
                    next.push(to);
                }
            }
            if backward {
                for from in relation.sources(&object).map_err(unavailable)? {
                    if let Some(facts) = relation.evidence.get(&(from.clone(), object.clone())) {
                        evidence.extend(facts.iter().cloned());
                    }
                    next.push(from);
                }
            }
            for found in next {
                reached.insert(found.clone());
                if *follow_chain && visited.insert(found.clone()) {
                    frontier.push(found);
                }
            }
        }
        reached.remove(anchor);
        let candidates = request
            .candidate_universe()
            .iter()
            .filter(|candidate| reached.contains(*candidate))
            .cloned()
            .collect();
        let mut unique = BTreeSet::new();
        evidence.retain(|item| unique.insert((item.source.clone(), item.locator.clone())));
        CompleteRelationshipSelection::try_new(request.clone(), candidates, evidence)
    }
}

impl RelationshipSelectionService for RelationRelationships {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        use crate::SnapshotBoundService as _;
        self.inner
            .as_ref()
            .map_or(&[], |inner| inner.source_snapshots())
    }

    fn select(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        if let Some(id) = request
            .query()
            .relationship()
            .as_str()
            .strip_prefix(RELATION_RELATIONSHIP_PREFIX)
        {
            return self.answer(id, request);
        }
        match &self.inner {
            Some(inner) => inner.select(request),
            None => Err(RelationshipSelectionError::Unavailable(
                "no relationship service is registered".into(),
            )),
        }
    }

    fn edges(
        &self,
        request: &RelationshipEdgesRequest,
    ) -> Result<CompleteRelationshipEdges, RelationshipSelectionError> {
        if request
            .relationship()
            .as_str()
            .starts_with(DERIVED_RELATIONSHIP_PREFIX)
        {
            return Err(RelationshipSelectionError::Unavailable(format!(
                "derived relationships are answered per object; the edges of `{}` are not listed",
                request.relationship().as_str()
            )));
        }
        match &self.inner {
            Some(inner) => inner.edges(request),
            None => Err(RelationshipSelectionError::Unavailable(
                "no relationship service is registered".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::SourceId;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("t", "model").unwrap(), local).unwrap()
    }

    fn serves() -> Relation {
        let mut relation = Relation::default();
        relation.relate(&id("p1"), &id("r1"), Vec::new());
        relation.relate(&id("p2"), &id("r1"), Vec::new());
        relation.relate(&id("p2"), &id("r2"), Vec::new());
        relation
    }

    #[test]
    fn pairs_answer_both_directions_and_undecided_ends_refuse() {
        let mut relation = serves();
        assert_eq!(relation.targets(&id("p2")).unwrap(), [id("r1"), id("r2")]);
        assert_eq!(relation.sources(&id("r1")).unwrap(), [id("p1"), id("p2")]);
        assert!(relation.targets(&id("r1")).unwrap().is_empty());
        relation.open_from(&id("p1"), "row 3 names `x`");
        assert_eq!(relation.targets(&id("p1")).unwrap_err(), "row 3 names `x`");
        assert!(relation.targets(&id("p2")).is_ok());
        relation.all_backward = Some("unknown".into());
        assert!(relation.sources(&id("r2")).is_err());
    }

    fn supplied(by: &serde_json::Value) -> RelationDefinition {
        serde_json::from_value(serde_json::json!({
            "id": "serves",
            "name": { "default": "Serves", "translations": {} },
            "from": { "kind": "all" },
            "to": { "kind": "all" },
            "by": by,
        }))
        .unwrap()
    }

    #[test]
    fn supplied_pairs_are_read_by_their_declared_headers_and_digested() {
        let definition = supplied(&serde_json::json!({ "kind": "supplied", "columns": [
            { "id": "from", "header": "Pump", "kind": "string" },
            { "id": "to", "header": "Room", "kind": "string" },
        ] }));
        let csv = b"Room,Pump\nR1,P1\n\nR2,P2\n";
        let pairs = SuppliedPairs::read(&definition, csv, "serves.csv", None).unwrap();
        assert_eq!(pairs.origin(), "serves.csv");
        assert_eq!(pairs.sha256(), crate::table_files::sha256_hex(csv));
        assert_eq!(
            listed(pairs.rows()),
            [
                (1, "P1".to_owned(), "R1".to_owned()),
                (2, "P2".to_owned(), "R2".to_owned())
            ]
        );
        // Other headers, and a relation listing its own pairs, are refused.
        assert!(
            SuppliedPairs::read(&definition, b"from,to\nP1,R1\n", "serves.csv", None)
                .unwrap_err()
                .contains("not declared")
        );
        let listed_relation = supplied(&serde_json::json!({
            "kind": "pairs", "pairs": { "type": "table", "value": [] },
        }));
        assert!(
            SuppliedPairs::read(&listed_relation, csv, "serves.csv", None)
                .unwrap_err()
                .contains("states its own pairs")
        );
        // Without declared columns the headers are `from` and `to`.
        let bare = supplied(&serde_json::json!({ "kind": "supplied" }));
        let pairs = SuppliedPairs::read(&bare, b"to,from\nR1,P1\n", "s.csv", None).unwrap();
        assert_eq!(
            listed(pairs.rows()),
            [(1, "P1".to_owned(), "R1".to_owned())]
        );
    }

    #[test]
    fn supplied_columns_name_exactly_from_and_to_as_text() {
        use crate::compiler::supplied_columns;
        let column = |id: &str, kind: &str| -> axioval_ir::contract::TableFileColumn {
            serde_json::from_value(serde_json::json!({ "id": id, "kind": kind })).unwrap()
        };
        assert!(
            supplied_columns(Some(&[column("from", "string"), column("to", "string")])).is_ok()
        );
        assert!(
            supplied_columns(Some(&[column("from", "string")]))
                .unwrap_err()
                .contains("no `to`")
        );
        assert!(
            supplied_columns(Some(&[column("from", "string"), column("to", "integer")]))
                .unwrap_err()
                .contains("not string")
        );
        assert!(
            supplied_columns(Some(&[column("from", "string"), column("room", "string")]))
                .unwrap_err()
                .contains("neither")
        );
    }

    #[test]
    fn identities_resolve_once_or_are_refused() {
        let mut index = BTreeMap::new();
        index.insert("A".to_owned(), vec![id("a")]);
        index.insert("B".to_owned(), vec![id("b1"), id("b2")]);
        assert_eq!(resolve(&index, "A", Some("guid")).unwrap(), id("a"));
        assert!(
            resolve(&index, "B", Some("guid"))
                .unwrap_err()
                .contains("names 2 objects")
        );
        assert_eq!(
            resolve(&index, "C", None).unwrap_err(),
            "`C` is no object of the model"
        );
    }
}
