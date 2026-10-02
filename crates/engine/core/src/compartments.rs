//! Compartments: groups of members joined across non-boundary elements
//! (`GroupingKey::Compartment`).
//!
//! Members join across a separating element the boundary selector does not
//! select, when they lie on opposite faces of it by the geometric adjacency
//! `axioval:derived.adjacent-across`, and where they touch directly without
//! lying on opposite faces of a boundary element. Each connected region of
//! sure joins is one compartment. A join that may or may not hold leaves
//! the compartments it could merge undecided, and a join the geometry
//! cannot read at all (an element whose spaces are unknown, a member whose
//! extent is unknown, a missing service) leaves every compartment
//! undecided: a compartment is never split by an assumption.

use std::collections::{BTreeMap, BTreeSet};

use axioval_ir::contract::{GroupingDefinition, Selector};
use axioval_ir::{NotEvaluatedReason, Object, ObjectId};

use crate::derived_relationships::{Derivation, DerivedRelationshipServiceHandle, across_side};
use crate::groupings::{Grouping, Membership, group_id};
use crate::pairwise::candidate_pairs;
use crate::proximity::{ProximityError, ProximityRequest, ProximityServiceHandle};
use crate::relationships::{
    RelationshipQuery, RelationshipSelectionRequest, SemanticRelationship, TraversalDirection,
};
use crate::{AdjacentSide, OutcomeRefiner, RuleContext, SelectorVerdict};

/// The adjacency defaults of a compartment declaration.
const TOLERANCE: f64 = 0.05;
const OVERLAP: f64 = 0.3;

/// How a separating element treats the members on its faces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Role {
    /// It separates them.
    Boundary,
    /// It joins them.
    Plain,
    /// It may do either.
    Undecided,
}

/// What the joins between members are.
#[derive(Default)]
struct Joins {
    sure: Vec<(ObjectId, ObjectId)>,
    possible: Vec<(ObjectId, ObjectId, String)>,
    /// Pairs on opposite faces of a boundary element.
    separated: BTreeSet<(ObjectId, ObjectId)>,
    /// Why a boundary element's faces are unknown, if one's are.
    unknown_boundary: Option<String>,
}

fn pair(a: &ObjectId, b: &ObjectId) -> (ObjectId, ObjectId) {
    if a <= b {
        (a.clone(), b.clone())
    } else {
        (b.clone(), a.clone())
    }
}

/// The compartments of `definition`, whose key holds the separators, the
/// boundary and the adjacency's tolerances.
pub(crate) fn derive(
    refiner: &dyn OutcomeRefiner,
    context: &RuleContext<'_>,
    definition: &GroupingDefinition,
    (separators, boundary): (&Selector, &Selector),
    (tolerance, overlap): (Option<f64>, Option<f64>),
) -> Grouping {
    let (tolerance, overlap) = (tolerance.unwrap_or(TOLERANCE), overlap.unwrap_or(OVERLAP));
    let mut members: Vec<ObjectId> = Vec::new();
    let mut undecided: Vec<(ObjectId, Membership)> = Vec::new();
    for object in context.project.objects() {
        match refiner.evaluate_selector(context, &definition.members, object) {
            SelectorVerdict::Match(_) => members.push(object.id.clone()),
            SelectorVerdict::NoMatch(_) => {}
            SelectorVerdict::Undecided(reason, why) => {
                undecided.push((object.id.clone(), Membership::Undecided(reason, why)));
            }
        }
    }
    let joined = joins(
        refiner,
        context,
        &members,
        (separators, boundary),
        (tolerance, overlap),
    )
    .and_then(|joins| contacts(context, &members, tolerance, joins));
    let joins = match joined {
        Ok(joins) => joins,
        Err((reason, why)) => {
            let every = members
                .into_iter()
                .map(|member| (member, Membership::Undecided(reason.clone(), why.clone())))
                .chain(undecided);
            return Grouping::of(&definition.id, every, &BTreeMap::new());
        }
    };
    let mut regions = Regions::new(&members);
    for (a, b) in &joins.sure {
        regions.join(a, b);
    }
    // Each region is keyed and identified by its least member.
    let mut keys = BTreeMap::new();
    let mut memberships = Vec::new();
    let mut group_of = BTreeMap::new();
    for member in &members {
        let least = regions.least(member);
        let group = match group_id(&least.source, &definition.id, &least.local_id) {
            Ok(group) => group,
            Err(why) => {
                memberships.push((
                    member.clone(),
                    Membership::Undecided(NotEvaluatedReason::InvalidEvidence, why),
                ));
                continue;
            }
        };
        keys.insert(group.clone(), least.local_id.clone());
        group_of.insert(member.clone(), group.clone());
        memberships.push((member.clone(), Membership::Grouped(group, Vec::new())));
    }
    memberships.extend(undecided);
    let mut grouping = Grouping::of(&definition.id, memberships, &keys);
    for (a, b, why) in &joins.possible {
        if let (Some(first), Some(second)) = (group_of.get(a), group_of.get(b))
            && first != second
        {
            let why = format!("{first} and {second} may be one compartment: {why}");
            grouping.undecide(first, why.clone());
            grouping.undecide(second, why);
        }
    }
    grouping
}

type Undecided = (NotEvaluatedReason, String);

/// The joins across every separating element.
fn joins(
    refiner: &dyn OutcomeRefiner,
    context: &RuleContext<'_>,
    members: &[ObjectId],
    (separators, boundary): (&Selector, &Selector),
    (tolerance, overlap): (f64, f64),
) -> Result<Joins, Undecided> {
    let Some(derived) = context.services.get::<DerivedRelationshipServiceHandle>() else {
        return Err((
            NotEvaluatedReason::MissingService,
            "no geometry derives which spaces lie beside each separating element".into(),
        ));
    };
    let identity = Derivation::AdjacentAcross {
        tolerance_metres: tolerance,
        overlap_metres: overlap,
    }
    .to_string();
    let relationship = SemanticRelationship::try_new(identity.clone())
        .map_err(|error| (NotEvaluatedReason::InvalidDeclaration, error.to_string()))?;
    let is_member: BTreeSet<&ObjectId> = members.iter().collect();
    let mut joins = Joins::default();
    for object in context.project.objects() {
        if is_member.contains(&object.id) {
            continue;
        }
        let Some(role) = role(refiner, context, (separators, boundary), object) else {
            continue;
        };
        let request = RelationshipSelectionRequest::try_new(
            object.id.clone(),
            members.to_vec(),
            RelationshipQuery::Related {
                relationship: relationship.clone(),
                direction: TraversalDirection::Forward,
                follow_chain: false,
            },
        )
        .map_err(|error| (NotEvaluatedReason::InvalidDeclaration, error.to_string()))?;
        let selection = match derived.select(&request) {
            Ok(selection) => selection,
            Err(error) if role == Role::Boundary => {
                joins.unknown_boundary.get_or_insert(format!(
                    "the spaces beside boundary element {} are unknown: {error}",
                    object.id
                ));
                continue;
            }
            Err(error) => {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "the spaces element {} may join are unknown: {error}",
                        object.id
                    ),
                ));
            }
        };
        let (mut plus, mut minus) = (Vec::new(), Vec::new());
        for space in selection.candidates() {
            for item in selection.evidence() {
                match across_side(&item.locator, &object.id, space) {
                    Some(AdjacentSide::Positive) => plus.push(space),
                    Some(AdjacentSide::Negative) => minus.push(space),
                    None => {}
                }
            }
        }
        for a in &plus {
            for b in minus.iter().filter(|b| *b != a) {
                match role {
                    Role::Boundary => {
                        joins.separated.insert(pair(a, b));
                    }
                    Role::Plain => joins.sure.push(((*a).clone(), (*b).clone())),
                    Role::Undecided => joins.possible.push((
                        (*a).clone(),
                        (*b).clone(),
                        format!("whether {} separates them is undecided", object.id),
                    )),
                }
            }
        }
    }
    Ok(joins)
}

/// Whether `object` separates members, joins them, or may; `None` when it
/// is surely no separating element.
fn role(
    refiner: &dyn OutcomeRefiner,
    context: &RuleContext<'_>,
    (separators, boundary): (&Selector, &Selector),
    object: &Object,
) -> Option<Role> {
    let separator = match refiner.evaluate_selector(context, separators, object) {
        SelectorVerdict::NoMatch(_) => return None,
        SelectorVerdict::Match(_) => true,
        SelectorVerdict::Undecided(..) => false,
    };
    Some(
        match (
            separator,
            refiner.evaluate_selector(context, boundary, object),
        ) {
            (true, SelectorVerdict::Match(_)) => Role::Boundary,
            (true, SelectorVerdict::NoMatch(_)) => Role::Plain,
            _ => Role::Undecided,
        },
    )
}

/// Adds the direct contacts between members to `joins`: members within
/// `tolerance` of each other join unless they lie on opposite faces of a
/// boundary element; a contact the measurement cannot settle may join.
fn contacts(
    context: &RuleContext<'_>,
    members: &[ObjectId],
    tolerance: f64,
    mut joins: Joins,
) -> Result<Joins, Undecided> {
    let Some(proximity) = context.services.get::<ProximityServiceHandle>() else {
        return Err((
            NotEvaluatedReason::MissingService,
            "no proximity service tells which spaces touch without an element between them".into(),
        ));
    };
    let mut bounds = Vec::new();
    for member in members {
        match proximity.bounds(member) {
            Ok(found) => bounds.push(found),
            // A bodiless member touches nothing.
            Err(ProximityError::NoBody) => {}
            Err(error) => {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("what space {member} touches is unknown: {error}"),
                ));
            }
        }
    }
    let pairs = candidate_pairs(&bounds, &bounds, tolerance)
        .map_err(|error| (NotEvaluatedReason::InvalidEvidence, error.to_string()))?;
    for candidate in pairs {
        let (a, b) = (candidate.subject(), candidate.counterpart());
        if joins.separated.contains(&pair(a, b)) {
            continue;
        }
        let measured = ProximityRequest::try_new(a.clone(), b.clone())
            .and_then(|request| proximity.measure_proximity(&request));
        let (lower, upper) = match measured {
            Ok(evidence) => evidence.separation_interval_metres(),
            Err(ProximityError::NoBody) => continue,
            Err(error) => {
                joins.possible.push((
                    a.clone(),
                    b.clone(),
                    format!("whether {a} and {b} touch is unknown: {error}"),
                ));
                continue;
            }
        };
        if lower > tolerance {
            continue;
        }
        if upper <= tolerance && joins.unknown_boundary.is_none() {
            joins.sure.push((a.clone(), b.clone()));
        } else {
            let why = joins
                .unknown_boundary
                .clone()
                .unwrap_or_else(|| format!("{a} and {b} lie between {lower} and {upper} m apart"));
            joins.possible.push((a.clone(), b.clone(), why));
        }
    }
    Ok(joins)
}

/// Connected regions of members, by union-find.
struct Regions {
    index: BTreeMap<ObjectId, usize>,
    parent: Vec<usize>,
    ids: Vec<ObjectId>,
}

impl Regions {
    fn new(members: &[ObjectId]) -> Self {
        let mut ids = members.to_vec();
        ids.sort();
        ids.dedup();
        Self {
            index: ids
                .iter()
                .cloned()
                .enumerate()
                .map(|(i, id)| (id, i))
                .collect(),
            parent: (0..ids.len()).collect(),
            ids,
        }
    }

    fn root(&mut self, mut at: usize) -> usize {
        while self.parent[at] != at {
            self.parent[at] = self.parent[self.parent[at]];
            at = self.parent[at];
        }
        at
    }

    /// Joins the regions of `a` and `b`; the lesser root stays, so a
    /// region's root is its least member.
    fn join(&mut self, a: &ObjectId, b: &ObjectId) {
        let (Some(&a), Some(&b)) = (self.index.get(a), self.index.get(b)) else {
            return;
        };
        let (a, b) = (self.root(a), self.root(b));
        let (low, high) = (a.min(b), a.max(b));
        self.parent[high] = low;
    }

    /// The least member of `member`'s region.
    fn least(&mut self, member: &ObjectId) -> ObjectId {
        let at = self.index[member];
        let root = self.root(at);
        self.ids[root].clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_are_keyed_by_their_least_member() {
        let source = axioval_ir::SourceId::new("t", "m").unwrap();
        let id = |local: &str| ObjectId::new(source.clone(), local).unwrap();
        let members = ["d", "a", "c", "b"].map(id);
        let mut regions = Regions::new(&members);
        regions.join(&id("d"), &id("c"));
        regions.join(&id("c"), &id("b"));
        assert_eq!(regions.least(&id("d")), id("b"));
        assert_eq!(regions.least(&id("a")), id("a"));
        regions.join(&id("b"), &id("a"));
        assert_eq!(regions.least(&id("d")), id("a"));
    }
}
