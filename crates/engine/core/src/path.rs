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

use std::collections::BTreeSet;

use axioval_ir::{Evidence, ObjectId};

use crate::derived_relationships::DERIVED_RELATIONSHIP_PREFIX;
use crate::relationships::{
    AbsentEndPolicy, RELATIONSHIP_KIND_PREFIX, RelationshipQuery, RelationshipSelectionError,
    RelationshipSelectionRequest, RelationshipSelectionServiceHandle, SemanticRelationship,
    TraversalDirection,
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
    /// completely.
    pub fn walk(
        &self,
        service: &RelationshipSelectionServiceHandle,
        from: &ObjectId,
        everything: &[ObjectId],
        scope: &[ObjectId],
        chain: bool,
        absent_ends: AbsentEndPolicy,
    ) -> Result<(BTreeSet<ObjectId>, Vec<Evidence>), RelationshipSelectionError> {
        let mut evidence = Vec::new();
        let mut hop = |anchor: &ObjectId,
                       universe: &[ObjectId],
                       relationship: &SemanticRelationship,
                       follow_chain: bool|
         -> Result<Vec<ObjectId>, RelationshipSelectionError> {
            let request = RelationshipSelectionRequest::try_new(
                anchor.clone(),
                universe.to_vec(),
                RelationshipQuery::Related {
                    relationship: relationship.clone(),
                    direction: self.direction,
                    follow_chain,
                },
            )?
            .with_absent_ends(absent_ends);
            let selection = service.select(&request)?;
            evidence.extend(selection.evidence().iter().cloned());
            Ok(selection.candidates().to_vec())
        };
        let mut reached = BTreeSet::new();
        if !chain || self.relationships.len() == 1 {
            for relationship in &self.relationships {
                reached.extend(hop(from, scope, relationship, chain)?);
            }
        } else {
            let mut seen = BTreeSet::from([from.clone()]);
            let mut frontier = vec![from.clone()];
            while let Some(current) = frontier.pop() {
                for relationship in &self.relationships {
                    for object in hop(&current, everything, relationship, true)? {
                        if seen.insert(object.clone()) {
                            frontier.push(object);
                        }
                    }
                }
            }
            seen.remove(from);
            let scope: BTreeSet<&ObjectId> = scope.iter().collect();
            reached.extend(seen.into_iter().filter(|object| scope.contains(object)));
        }
        reached.remove(from);
        Ok((reached, evidence))
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
}
