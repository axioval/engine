//! The step grammar every relationship path shares, and its walk.
//!
//! A step is `Relationship[|Relationship…][:direction][+]`:
//!
//! - one or more relationship identities separated by `|`, any of which the
//!   step may take;
//! - an optional direction, `forward` (the default), `backward` or `either`,
//!   applying to every alternative, written once after the last one;
//! - an optional trailing `+`, taking the step one or more times.
//!
//! A step through one relationship with `+` follows that relationship's
//! chain as the source answers it. A step through several with `+` mixes
//! them along the chain: each hop takes any of them, so
//! `IfcRelFillsElement|IfcRelVoidsElement:backward+` climbs from a door
//! through the opening it fills to the wall the opening voids. Where the
//! chain changes relationship it passes through objects of the project.
//!
//! A derived identity (`axioval:derived.…`) or a relationship kind
//! (`axioval:relationship.…`) holds a colon of its own, so in
//! the last alternative only a colon followed by a direction word ends it.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use axioval_ir::{Evidence, ObjectId};

use crate::derived_relationships::DERIVED_RELATIONSHIP_PREFIX;
use crate::relationships::{
    AbsentEndPolicy, CandidateUniverse, RELATIONSHIP_KIND_PREFIX, RelationshipQuery,
    RelationshipSelectionError, RelationshipSelectionRequest, RelationshipSelectionServiceHandle,
    SemanticRelationship, TraversalDirection,
};

const DIRECTIONS: [&str; 3] = ["forward", "backward", "either"];

/// One parsed step of a relationship path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathSegment {
    relationships: Vec<SemanticRelationship>,
    direction: TraversalDirection,
    chain: bool,
}

impl PathSegment {
    /// Parses `Relationship[|Relationship…][:direction][+]`.
    ///
    /// # Errors
    ///
    /// Returns why the text is no step: an empty alternative, one named
    /// twice, a direction other than `forward`, `backward` or `either`, or a
    /// direction written after an alternative other than the last.
    pub fn parse(text: &str) -> Result<Self, String> {
        let trimmed = text.trim();
        let (body, chain) = match trimmed.strip_suffix('+') {
            Some(body) => (body, true),
            None => (trimmed, false),
        };
        let mut alternatives: Vec<&str> = body.split('|').collect();
        let last = alternatives.pop().unwrap_or_default();
        let derived = [DERIVED_RELATIONSHIP_PREFIX, RELATIONSHIP_KIND_PREFIX]
            .iter()
            .any(|prefix| last.trim().starts_with(prefix));
        let (last, stated) = match last.rsplit_once(':') {
            Some((_, stated)) if derived && !DIRECTIONS.contains(&stated.trim()) => (last, None),
            Some((relationship, stated)) => (relationship, Some(stated.trim())),
            None => (last, None),
        };
        let direction = match stated {
            None | Some("forward") => TraversalDirection::Forward,
            Some("backward") => TraversalDirection::Backward,
            Some("either") => TraversalDirection::Either,
            Some(other) => return Err(format!("direction `{other}` is unsupported")),
        };
        alternatives.push(last);
        let mut relationships: Vec<SemanticRelationship> = Vec::new();
        for alternative in alternatives {
            let alternative = alternative.trim();
            if let Some((_, stated)) = alternative.rsplit_once(':')
                && DIRECTIONS.contains(&stated.trim())
            {
                return Err(format!(
                    "step `{trimmed}` states a direction inside `{alternative}`; one direction \
                     follows the last alternative and applies to all"
                ));
            }
            let relationship = SemanticRelationship::try_new(alternative)
                .map_err(|_| format!("step `{trimmed}` names an empty relationship"))?;
            if relationships.contains(&relationship) {
                return Err(format!(
                    "step `{trimmed}` names `{alternative}` more than once"
                ));
            }
            relationships.push(relationship);
        }
        Ok(Self {
            relationships,
            direction,
            chain,
        })
    }

    /// The relationships the step may take, in the order written.
    #[must_use]
    pub fn relationships(&self) -> &[SemanticRelationship] {
        &self.relationships
    }

    /// The direction every alternative is taken in.
    #[must_use]
    pub fn direction(&self) -> TraversalDirection {
        self.direction
    }

    /// Whether the step is taken one or more times (a trailing `+`).
    #[must_use]
    pub fn chain(&self) -> bool {
        self.chain
    }

    /// The same step taken the other way.
    #[must_use]
    pub fn reversed(&self) -> Self {
        Self {
            relationships: self.relationships.clone(),
            direction: match self.direction {
                TraversalDirection::Forward => TraversalDirection::Backward,
                TraversalDirection::Backward => TraversalDirection::Forward,
                TraversalDirection::Either => TraversalDirection::Either,
            },
            chain: self.chain,
        }
    }

    /// How messages name the step's relationships: `A` or `A|B`.
    #[must_use]
    pub fn shown(&self) -> String {
        self.relationships
            .iter()
            .map(SemanticRelationship::as_str)
            .collect::<Vec<_>>()
            .join("|")
    }

    /// The objects of `scope` the step reaches from `from`, taken once or,
    /// with `chain`, one or more times, with the service's evidence.
    ///
    /// A chain through several relationships hops between objects of
    /// `everything`, each hop following any alternative's own chain, and
    /// keeps what lies in `scope`. `from` is never among the result.
    ///
    /// # Errors
    ///
    /// Returns the service's error for the first query it cannot answer
    /// completely, and [`RelationshipSelectionError::OverBudget`] once its
    /// answers hold more than [`WALK_BUDGET`] objects and citations.
    pub fn walk(
        &self,
        service: &RelationshipSelectionServiceHandle,
        from: &ObjectId,
        everything: &[ObjectId],
        scope: &[ObjectId],
        chain: bool,
        absent_ends: AbsentEndPolicy,
    ) -> Result<(BTreeSet<ObjectId>, Vec<Evidence>), RelationshipSelectionError> {
        // Each universe is refused only where a request would carry it: a
        // single hop asks over `scope`, a mixed chain over `everything`
        // and keeps what lies in `scope`.
        let (everything, scope) = if !chain || self.relationships.len() == 1 {
            let scope = CandidateUniverse::try_new(scope.to_vec())?;
            (scope.clone(), scope)
        } else {
            let mut kept = scope.to_vec();
            kept.sort();
            kept.dedup();
            (
                CandidateUniverse::try_new(everything.to_vec())?,
                CandidateUniverse::try_new(kept)?,
            )
        };
        self.walk_within(service, from, &everything, &scope, chain, absent_ends)
    }

    /// [`Self::walk`] over universes built already: every request it makes
    /// shares them, so a walk costs what it reaches, never a copy of
    /// `everything` per hop. A caller walking from many objects builds the
    /// universes once.
    ///
    /// # Errors
    ///
    /// As [`Self::walk`].
    pub fn walk_within(
        &self,
        service: &RelationshipSelectionServiceHandle,
        from: &ObjectId,
        everything: &CandidateUniverse,
        scope: &CandidateUniverse,
        chain: bool,
        absent_ends: AbsentEndPolicy,
    ) -> Result<(BTreeSet<ObjectId>, Vec<Evidence>), RelationshipSelectionError> {
        let walk = Walk {
            step: self,
            service,
            everything,
            scope,
            chain,
            absent_ends,
            closed: false,
            memo: None,
        };
        let mut evidence = Vec::new();
        let reached = walk.from(from, &mut Budget::new(WALK_BUDGET), &mut evidence)?;
        Ok((reached, evidence))
    }

    /// The objects of `scope` the step reaches from any of `starts`: the
    /// union of [`Self::walk_within`] from each, its evidence the union of
    /// theirs, in no particular order and possibly repeated (a caller sorts
    /// and deduplicates it).
    ///
    /// Where the service closes the chains of every relationship the step
    /// takes ([`RelationshipSelectionServiceHandle::closes_chains`]), a
    /// chain is never asked from an object a chain of the same relationship
    /// already reached, and no start is walked from that a walk already
    /// reached (unless only it could reach that walk's start, in `scope` and
    /// not reached yet): the answers could add nothing.
    /// Each object is then asked about once per relationship at most, where
    /// asking every chain again took the square of the objects reached
    /// (#428). Otherwise every walk asks every chain, as
    /// [`Self::walk_within`] does. Either way a request asked from two
    /// starts is answered once.
    ///
    /// # Errors
    ///
    /// As [`Self::walk`]: [`RelationshipSelectionError::OverBudget`] once
    /// the answers to all its starts hold more than [`WALK_BUDGET`].
    pub fn walk_from_each<'o>(
        &self,
        service: &RelationshipSelectionServiceHandle,
        starts: impl IntoIterator<Item = &'o ObjectId>,
        everything: &CandidateUniverse,
        scope: &CandidateUniverse,
        chain: bool,
        absent_ends: AbsentEndPolicy,
    ) -> Result<(BTreeSet<ObjectId>, Vec<Evidence>), RelationshipSelectionError> {
        self.walk_from_each_within(
            service,
            starts,
            (everything, scope),
            chain,
            absent_ends,
            WALK_BUDGET,
        )
    }

    /// [`Self::walk_from_each`] within a budget of `limit`.
    fn walk_from_each_within<'o>(
        &self,
        service: &RelationshipSelectionServiceHandle,
        starts: impl IntoIterator<Item = &'o ObjectId>,
        (everything, scope): (&CandidateUniverse, &CandidateUniverse),
        chain: bool,
        absent_ends: AbsentEndPolicy,
        limit: usize,
    ) -> Result<(BTreeSet<ObjectId>, Vec<Evidence>), RelationshipSelectionError> {
        let walk = Walk {
            step: self,
            service,
            everything,
            scope,
            chain,
            absent_ends,
            closed: chain
                && self
                    .relationships
                    .iter()
                    .all(|relationship| service.closes_chains(relationship)),
            memo: Some(RefCell::default()),
        };
        let mut budget = Budget::new(limit);
        let mut evidence = Vec::new();
        let mut reached = BTreeSet::new();
        // The starts whose walks reached each object: a walk from it reaches
        // nothing outside theirs and those of their starts in `scope`.
        let mut reached_from: BTreeMap<ObjectId, Vec<ObjectId>> = BTreeMap::new();
        for start in starts {
            if walk.closed
                && reached_from.get(start).is_some_and(|walked| {
                    walked
                        .iter()
                        .any(|from| reached.contains(from) || !scope.contains(from))
                })
            {
                continue;
            }
            let found = walk.from(start, &mut budget, &mut evidence)?;
            if walk.closed {
                for object in &found {
                    reached_from
                        .entry(object.clone())
                        .or_default()
                        .push(start.clone());
                }
            }
            reached.extend(found);
        }
        Ok((reached, evidence))
    }
}

/// How many objects and citations the answers to one walk of a step may
/// hold together, over every start and every query, each query counting
/// one more, before the walk is refused
/// ([`RelationshipSelectionError::OverBudget`], #428). Asking every chain
/// again from each of 1,500 objects read 1.1 million per walk in 1.8 s
/// (release); closed, the same walk reads 4,600. The budget stops a walk
/// after about ten seconds and leaves a closed walk over a project of a
/// million objects room.
pub const WALK_BUDGET: usize = 5_000_000;

/// What a walk has read so far, and may read.
struct Budget {
    read: usize,
    queries: usize,
    limit: usize,
}

impl Budget {
    fn new(limit: usize) -> Self {
        Self {
            read: 0,
            queries: 0,
            limit,
        }
    }
}

/// The answers a walk was given, by anchor and the index of the
/// relationship asked.
type Memo = RefCell<BTreeMap<(ObjectId, usize), Vec<ObjectId>>>;

/// One walk of a step: its service, universes and how it may skip.
struct Walk<'w> {
    step: &'w PathSegment,
    service: &'w RelationshipSelectionServiceHandle,
    everything: &'w CandidateUniverse,
    scope: &'w CandidateUniverse,
    chain: bool,
    absent_ends: AbsentEndPolicy,
    /// Whether the service closes the chains of every relationship.
    closed: bool,
    /// The answers already asked for, by anchor and relationship, where the
    /// walk goes from several starts: one request is answered once.
    memo: Option<Memo>,
}

impl Walk<'_> {
    /// One query of the step's `index`th relationship, its evidence kept and
    /// its answer charged to `budget`; a query asked before is answered from
    /// the memo, its evidence kept already.
    fn hop(
        &self,
        anchor: &ObjectId,
        index: usize,
        budget: &mut Budget,
        evidence: &mut Vec<Evidence>,
    ) -> Result<Vec<ObjectId>, RelationshipSelectionError> {
        let key = (anchor.clone(), index);
        let known = self
            .memo
            .as_ref()
            .and_then(|memo| memo.borrow().get(&key).cloned());
        if let Some(answer) = known {
            self.charge(budget, anchor, answer.len())?;
            return Ok(answer);
        }
        // A single hop, or a chain through one relationship, asks over the
        // scope; a chain mixing several over everything.
        let mixed = self.chain && self.step.relationships.len() > 1;
        let universe = if mixed { self.everything } else { self.scope };
        let request = RelationshipSelectionRequest::within(
            anchor.clone(),
            universe.clone(),
            RelationshipQuery::Related {
                relationship: self.step.relationships[index].clone(),
                direction: self.step.direction,
                follow_chain: self.chain,
            },
        )
        .with_absent_ends(self.absent_ends);
        let selection = self.service.select(&request)?;
        self.charge(
            budget,
            anchor,
            selection.candidates().len() + selection.evidence().len(),
        )?;
        evidence.extend(selection.evidence().iter().cloned());
        let answer = selection.candidates().to_vec();
        if let Some(memo) = &self.memo {
            memo.borrow_mut().insert(key, answer.clone());
        }
        Ok(answer)
    }

    /// Charges one query reading `read` objects and citations to `budget`,
    /// refusing the walk past its limit.
    fn charge(
        &self,
        budget: &mut Budget,
        anchor: &ObjectId,
        read: usize,
    ) -> Result<(), RelationshipSelectionError> {
        budget.queries += 1;
        budget.read += 1 + read;
        if budget.read > budget.limit {
            return Err(RelationshipSelectionError::OverBudget(format!(
                "the step `{}:{}{}` read {} related objects and citations in {} queries, \
                 the last from {anchor}, past the budget of {}",
                self.step.shown(),
                match self.step.direction {
                    TraversalDirection::Forward => "forward",
                    TraversalDirection::Backward => "backward",
                    TraversalDirection::Either => "either",
                },
                if self.step.chain { "+" } else { "" },
                budget.read,
                budget.queries,
                budget.limit,
            )));
        }
        Ok(())
    }

    /// The objects of `scope` the step reaches from `from`, `from` left out.
    fn from(
        &self,
        from: &ObjectId,
        budget: &mut Budget,
        evidence: &mut Vec<Evidence>,
    ) -> Result<BTreeSet<ObjectId>, RelationshipSelectionError> {
        let relationships = &self.step.relationships;
        let mut reached = BTreeSet::new();
        if !self.chain || relationships.len() == 1 {
            for index in 0..relationships.len() {
                reached.extend(self.hop(from, index, budget, evidence)?);
            }
        } else {
            let mut seen = BTreeSet::from([from.clone()]);
            let mut frontier = vec![from.clone()];
            // Per relationship, the objects a chain of it already reached: a
            // closed chain from one of them reaches only what is seen.
            let mut covered = vec![BTreeSet::new(); relationships.len()];
            while let Some(current) = frontier.pop() {
                for (index, covered) in covered.iter_mut().enumerate() {
                    if self.closed && covered.contains(&current) {
                        continue;
                    }
                    for object in self.hop(&current, index, budget, evidence)? {
                        if self.closed {
                            covered.insert(object.clone());
                        }
                        if seen.insert(object.clone()) {
                            frontier.push(object);
                        }
                    }
                }
            }
            seen.remove(from);
            reached.extend(
                seen.into_iter()
                    .filter(|object| self.scope.contains(object)),
            );
        }
        reached.remove(from);
        Ok(reached)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(step: &PathSegment) -> Vec<&str> {
        step.relationships()
            .iter()
            .map(SemanticRelationship::as_str)
            .collect()
    }

    #[test]
    fn a_step_names_one_or_several_relationships() {
        let step = PathSegment::parse("IfcRelAggregates").unwrap();
        assert_eq!(names(&step), ["IfcRelAggregates"]);
        assert_eq!(step.direction(), TraversalDirection::Forward);
        assert!(!step.chain());

        let step =
            PathSegment::parse(" IfcRelFillsElement | IfcRelVoidsElement:backward+ ").unwrap();
        assert_eq!(names(&step), ["IfcRelFillsElement", "IfcRelVoidsElement"]);
        assert_eq!(step.direction(), TraversalDirection::Backward);
        assert!(step.chain());
        assert_eq!(step.shown(), "IfcRelFillsElement|IfcRelVoidsElement");
        assert_eq!(step.reversed().direction(), TraversalDirection::Forward);
    }

    #[test]
    fn derived_identities_keep_their_colons() {
        let step = PathSegment::parse("axioval:derived.adjacent-space;reach=1").unwrap();
        assert_eq!(names(&step), ["axioval:derived.adjacent-space;reach=1"]);
        let step =
            PathSegment::parse("IfcRelNests|axioval:derived.same-level;by=name:either").unwrap();
        assert_eq!(
            names(&step),
            ["IfcRelNests", "axioval:derived.same-level;by=name"]
        );
        assert_eq!(step.direction(), TraversalDirection::Either);
        let step = PathSegment::parse("axioval:relationship.containment").unwrap();
        assert_eq!(names(&step), ["axioval:relationship.containment"]);
        assert_eq!(step.direction(), TraversalDirection::Forward);
        let step = PathSegment::parse("axioval:relationship.fills:backward").unwrap();
        assert_eq!(names(&step), ["axioval:relationship.fills"]);
        assert_eq!(step.direction(), TraversalDirection::Backward);
        let step = PathSegment::parse("axioval:derived.intersects|IfcRelNests").unwrap();
        assert_eq!(names(&step), ["axioval:derived.intersects", "IfcRelNests"]);
    }

    #[test]
    fn malformed_steps_are_refused() {
        for text in [
            "",
            "IfcRelNests|",
            "|IfcRelNests",
            "IfcRelNests||IfcRelAggregates",
            "IfcRelNests|IfcRelNests",
            "IfcRelNests:sideways",
            "IfcRelNests:backward|IfcRelAggregates",
            "IfcRelNests:backward|IfcRelAggregates:backward",
        ] {
            assert!(PathSegment::parse(text).is_err(), "{text:?} parsed");
        }
    }

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axioval_ir::SourceId;

    use crate::relationships::{CompleteRelationshipSelection, RelationshipSelectionService};

    fn object(n: usize) -> ObjectId {
        ObjectId::new(SourceId::new("t", "model").unwrap(), format!("n{n:02}")).unwrap()
    }

    /// Edges by relationship, answered as the IFC adapter answers them: a
    /// chain is a breadth-first closure citing every edge it crossed, the
    /// universe only filtering the answer.
    struct Graph {
        edges: BTreeMap<&'static str, Vec<(usize, usize)>>,
        closed: bool,
        asked: AtomicUsize,
    }

    impl RelationshipSelectionService for Graph {
        fn select(
            &self,
            request: &RelationshipSelectionRequest,
        ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
            self.asked.fetch_add(1, Ordering::Relaxed);
            let RelationshipQuery::Related {
                relationship,
                direction,
                follow_chain,
            } = request.query()
            else {
                return Err(RelationshipSelectionError::InvalidRequest);
            };
            let edges = &self.edges[relationship.as_str()];
            let source = request.anchor().source.clone();
            let mut evidence = vec![Evidence::exact(
                source.clone(),
                format!("scan:{}", relationship.as_str()),
            )];
            let mut reached = BTreeSet::new();
            let mut seen = BTreeSet::from([request.anchor().clone()]);
            let mut frontier = vec![request.anchor().clone()];
            while let Some(current) = frontier.pop() {
                for &(from, to) in edges {
                    let (from, to) = (object(from), object(to));
                    let next = match direction {
                        TraversalDirection::Forward | TraversalDirection::Either
                            if from == current =>
                        {
                            to.clone()
                        }
                        TraversalDirection::Backward | TraversalDirection::Either
                            if to == current =>
                        {
                            from.clone()
                        }
                        _ => continue,
                    };
                    let cited = format!("edge:{}:{from}->{to}", relationship.as_str());
                    if !evidence.iter().any(|item| item.locator == cited) {
                        evidence.push(Evidence::exact(source.clone(), cited));
                    }
                    reached.insert(next.clone());
                    if *follow_chain && seen.insert(next.clone()) {
                        frontier.push(next);
                    }
                }
            }
            let candidates = reached
                .into_iter()
                .filter(|candidate| {
                    candidate != request.anchor() && request.universe().contains(candidate)
                })
                .collect();
            CompleteRelationshipSelection::try_new(request.clone(), candidates, evidence)
        }

        fn closes_chains(&self, _relationship: &SemanticRelationship) -> bool {
            self.closed
        }
    }

    /// Thirty objects: `a` joins each even one to the next, `b` each odd
    /// one, and `a` also every fifth to the one five on, so a chain of
    /// either reaches only a stretch and the two together reach them all.
    fn graph(closed: bool) -> (Arc<Graph>, RelationshipSelectionServiceHandle) {
        let a = (0..29)
            .filter(|n| n % 2 == 0)
            .map(|n| (n, n + 1))
            .chain((0..25).step_by(5).map(|n| (n, n + 5)))
            .collect();
        let b = (0..29).filter(|n| n % 2 == 1).map(|n| (n, n + 1)).collect();
        let graph = Arc::new(Graph {
            edges: BTreeMap::from([("a", a), ("b", b)]),
            closed,
            asked: AtomicUsize::new(0),
        });
        let handle = RelationshipSelectionServiceHandle::new(graph.clone());
        (graph, handle)
    }

    fn universe(objects: impl IntoIterator<Item = usize>) -> CandidateUniverse {
        CandidateUniverse::try_new(objects.into_iter().map(object).collect()).unwrap()
    }

    /// The walk and its evidence, sorted and unrepeated, and the queries
    /// asked.
    fn walked(
        step: &str,
        closed: bool,
        starts: &[usize],
        scope: &CandidateUniverse,
    ) -> ((BTreeSet<ObjectId>, Vec<String>), usize) {
        let (graph, handle) = graph(closed);
        let starts: Vec<ObjectId> = starts.iter().copied().map(object).collect();
        let step = PathSegment::parse(step).unwrap();
        let (reached, evidence) = step
            .walk_from_each(
                &handle,
                &starts,
                &universe(0..30),
                scope,
                step.chain(),
                AbsentEndPolicy::Refuse,
            )
            .unwrap();
        let cited: BTreeSet<String> = evidence.into_iter().map(|item| item.locator).collect();
        (
            (reached, cited.into_iter().collect()),
            graph.asked.load(Ordering::Relaxed),
        )
    }

    #[test]
    fn closed_chains_are_asked_once_and_answer_as_every_chain_does() {
        let everything = universe(0..30);
        let odd = universe((0..30).filter(|n| n % 2 == 1));
        for (step, starts, scope) in [
            ("a|b:either+", &[3, 0, 17][..], &everything),
            ("a|b:forward+", &[0, 1, 10, 2], &odd),
            ("a|b:backward+", &[29, 12], &everything),
            ("a:either+", &[0, 1, 5, 6, 10, 11], &everything),
            ("b:forward+", &[1, 3, 4, 7], &odd),
            ("a|b:either", &[4, 9], &everything),
        ] {
            let (every, asked_every) = walked(step, false, starts, scope);
            let (closed, asked_closed) = walked(step, true, starts, scope);
            assert_eq!(closed, every, "{step} from {starts:?}");
            assert!(asked_closed <= asked_every, "{step}");
            // The union of single walks, as each start walked alone.
            let (_, handle) = graph(false);
            let step_ = PathSegment::parse(step).unwrap();
            let mut union = BTreeSet::new();
            for start in starts {
                let (found, _) = step_
                    .walk_within(
                        &handle,
                        &object(*start),
                        &everything,
                        scope,
                        step_.chain(),
                        AbsentEndPolicy::Refuse,
                    )
                    .unwrap();
                union.extend(found);
            }
            assert_eq!(closed.0, union, "{step} from {starts:?}");
        }
        // Mixing the two either way asks each chain from every object reached,
        // once for all three starts: 60 queries. Closed, an object a chain of `a`
        // reached is asked only about `b`, and the other way round.
        let (_, asked_every) = walked("a|b:either+", false, &[3, 0, 17], &everything);
        let (_, asked_closed) = walked("a|b:either+", true, &[3, 0, 17], &everything);
        assert_eq!(asked_every, 60);
        assert_eq!(asked_closed, 29);
    }

    #[test]
    fn a_walk_past_its_budget_is_refused_by_name() {
        let (_, handle) = graph(false);
        let step = PathSegment::parse("a|b:either+").unwrap();
        let everything = universe(0..30);
        let starts = [object(0)];
        let walk = |limit| {
            step.walk_from_each_within(
                &handle,
                &starts,
                (&everything, &everything),
                true,
                AbsentEndPolicy::Refuse,
                limit,
            )
        };
        assert!(walk(WALK_BUDGET).is_ok());
        let Err(RelationshipSelectionError::OverBudget(why)) = walk(100) else {
            panic!("a walk past its budget answered");
        };
        assert!(
            why.starts_with("the step `a|b:either+` read ")
                && why.ends_with("past the budget of 100"),
            "{why}"
        );
    }
}
