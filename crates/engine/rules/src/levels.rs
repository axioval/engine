//! Containers of several sources matched as one level
//! (`axioval:derived.same-level`), for the container modes of
//! `property-comparison` and `same-container`.
//!
//! A container is always on its own level, and two containers of one source
//! never share one: a source states its own storeys. Across sources, two
//! containers are one level when [`LevelMatch`] says so from the rule's
//! `level_property` of each, a length for an elevation match or text for a
//! name match. A container that does not state it leaves the pair
//! undecided, never "another level".

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{LevelFacts, LevelMatch, NotEvaluatedReason, RuleContext};
use axioval_ir::{Evidence, ObjectId, PropertyValue, QuantityDimension};

use crate::support::{PropertyRef, Resolved, Unavailable, invalid, resolve};

/// Whether two sets of containers share a level, or name the same levels,
/// with the facts that decided it.
type Decided = Result<(bool, Vec<Evidence>), Unavailable>;

/// A level match with the facts of every container it read, read once.
pub(crate) struct Levels<'a> {
    matcher: LevelMatch,
    property: PropertyRef<'a>,
    facts: BTreeMap<ObjectId, Result<(LevelFacts, Vec<Evidence>), Unavailable>>,
}

impl<'a> Levels<'a> {
    /// The match `relationship` names, comparing `property`.
    ///
    /// # Errors
    ///
    /// An invalid declaration for anything but a valid
    /// `axioval:derived.same-level` identity.
    pub(crate) fn parse(
        relationship: &str,
        property: PropertyRef<'a>,
    ) -> Result<Self, Unavailable> {
        match LevelMatch::parse(relationship) {
            Ok(Some(matcher)) => Ok(Self {
                matcher,
                property,
                facts: BTreeMap::new(),
            }),
            Ok(None) => Err(invalid(format!(
                "`container_relationship` `{relationship}` is not `axioval:derived.same-level`"
            ))),
            Err(_) => Err(invalid(format!(
                "`container_relationship` `{relationship}` has an invalid parameter"
            ))),
        }
    }

    fn facts(
        &mut self,
        context: &RuleContext<'_>,
        container: &ObjectId,
    ) -> Result<(LevelFacts, Vec<Evidence>), Unavailable> {
        let (matcher, property) = (self.matcher, self.property);
        self.facts
            .entry(container.clone())
            .or_insert_with(|| read(context, matcher, property, container))
            .clone()
    }

    /// Whether `left` and `right` are one level: `None` when undecided.
    fn same(
        &mut self,
        context: &RuleContext<'_>,
        left: &ObjectId,
        right: &ObjectId,
        evidence: &mut Vec<Evidence>,
    ) -> Result<bool, Unavailable> {
        if left == right {
            return Ok(true);
        }
        if left.source == right.source {
            return Ok(false);
        }
        let (left_facts, left_cited) = self.facts(context, left)?;
        let (right_facts, right_cited) = self.facts(context, right)?;
        let same = self.matcher.same(&left_facts, &right_facts).ok_or_else(|| {
            (
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "whether {left} and {right} are one level is unknown: one does not state {}",
                    self.property
                ),
            )
        })?;
        evidence.extend(left_cited);
        evidence.extend(right_cited);
        if same {
            evidence.push(Evidence::exact(
                left.source.clone(),
                format!("{}:{left}={right}", self.matcher),
            ));
        }
        Ok(same)
    }

    /// Whether `mine` and `theirs` share a level: one pair surely on one
    /// level is enough; otherwise an undecided pair leaves it undecided.
    pub(crate) fn overlap(
        &mut self,
        context: &RuleContext<'_>,
        mine: &BTreeSet<ObjectId>,
        theirs: &BTreeSet<ObjectId>,
    ) -> Decided {
        let mut evidence = Vec::new();
        let mut undecided = None;
        for left in mine {
            for right in theirs {
                match self.same(context, left, right, &mut evidence) {
                    Ok(true) => return Ok((true, evidence)),
                    Ok(false) => {}
                    Err(unavailable) => {
                        undecided.get_or_insert(unavailable);
                    }
                }
            }
        }
        match undecided {
            Some(unavailable) => Err(unavailable),
            None => Ok((false, evidence)),
        }
    }

    /// Whether `mine` and `theirs` name the same levels: every container of
    /// each shares a level with one of the other. A container surely on no
    /// level of the other decides "no"; an undecided one otherwise leaves
    /// it undecided.
    pub(crate) fn equivalent(
        &mut self,
        context: &RuleContext<'_>,
        mine: &BTreeSet<ObjectId>,
        theirs: &BTreeSet<ObjectId>,
    ) -> Decided {
        let mut evidence = Vec::new();
        let mut undecided = None;
        for (from, to) in [(mine, theirs), (theirs, mine)] {
            for container in from {
                let one = BTreeSet::from([container.clone()]);
                match self.overlap(context, &one, to) {
                    Ok((true, cited)) => evidence.extend(cited),
                    Ok((false, cited)) => {
                        evidence.extend(cited);
                        return Ok((false, evidence));
                    }
                    Err(unavailable) => {
                        undecided.get_or_insert(unavailable);
                    }
                }
            }
        }
        match undecided {
            Some(unavailable) => Err(unavailable),
            None => Ok((true, evidence)),
        }
    }
}

/// The level match a rule declares: `container_relationship` and the
/// `level_property` it compares, both or neither.
///
/// # Errors
///
/// An invalid declaration for one without the other, or an identity that is
/// not a valid `axioval:derived.same-level`.
pub(crate) fn declared<'a>(
    relationship: Option<&'a str>,
    property: Option<PropertyRef<'a>>,
) -> Result<Option<(&'a str, PropertyRef<'a>)>, Unavailable> {
    match (relationship, property) {
        (None, None) => Ok(None),
        (Some(relationship), Some(property)) => {
            Levels::parse(relationship, property)?;
            Ok(Some((relationship, property)))
        }
        (Some(_), None) => Err(invalid(
            "`container_relationship` needs `level_property`, the property levels are matched by",
        )),
        (None, Some(_)) => Err(invalid(
            "`level_property` applies with `container_relationship` only",
        )),
    }
}

/// What `container` states for `matcher`, with the evidence of it. An
/// absent fact is `None`; a fact of another kind is refused.
fn read(
    context: &RuleContext<'_>,
    matcher: LevelMatch,
    property: PropertyRef<'_>,
    container: &ObjectId,
) -> Result<(LevelFacts, Vec<Evidence>), Unavailable> {
    let Some(object) = context.project.object(container) else {
        return Err((
            NotEvaluatedReason::InvalidEvidence,
            format!("container {container} is not in the project"),
        ));
    };
    let resolved = resolve(context, object, property)?;
    let evidence = match &resolved {
        Resolved::Present(property) => property.evidence.iter().cloned().collect(),
        Resolved::Absent(evidence) => vec![evidence.clone()],
    };
    let mut facts = LevelFacts::default();
    match (matcher, resolved.value()) {
        (_, None | Some(PropertyValue::Null)) => {}
        (
            LevelMatch::Elevation { .. },
            Some(PropertyValue::Quantity {
                value,
                dimension: QuantityDimension::Length,
            }),
        ) => facts.elevation_metres = Some(*value),
        (LevelMatch::Name, Some(PropertyValue::String(text))) => {
            facts.name = Some(text.clone());
        }
        (_, Some(_)) => {
            return Err((
                NotEvaluatedReason::InvalidEvidence,
                format!(
                    "{property} of level {container} is not a {}",
                    match matcher {
                        LevelMatch::Elevation { .. } => "length",
                        LevelMatch::Name => "text",
                    }
                ),
            ));
        }
    }
    Ok((facts, evidence))
}
