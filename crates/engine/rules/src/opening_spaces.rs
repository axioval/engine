//! The spaces a door, window or opening connects, by its host wall's
//! declared exposure.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    AdjacentSide, CapabilityEvaluation, CompiledRule, Derivation, NotEvaluatedReason,
    ParameterDescriptor, ParameterType, RuleCapability, RuleContext, SemanticRelationship,
    TraversalDirection, adjacent_side,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Finding, Object, ObjectId, PropertyValue, Scope, SourceId};

use crate::counts::Population;
use crate::pairs::severity;
use crate::selection::select_objects;
use crate::support::{
    Parameters, PropertyRef, Resolved, Traversal, Unavailable, finding, invalid, resolve,
};

/// Requires each selected door, window or opening to relate to the spaces
/// its host wall calls for: two, one on each side, in an internal wall; one
/// in an external wall, the other side being outside.
///
/// The host is what `host_path` reaches among the `host_selector` objects
/// (with IFC, `IfcRelFillsElement` then `IfcRelVoidsElement` backward from a
/// door or window, `IfcRelVoidsElement` backward from an opening). Its
/// boolean `external_property` decides internal or external; a host that
/// does not declare it, or hosts that disagree, leave the element not
/// evaluated. The spaces are what `space_path` reaches among the
/// `space_selector` objects (every object by default): a relationship the
/// model states, or `axioval:derived.adjacent-space`, whose evidence records
/// which face of the element each space lies on, so two spaces on the same
/// face are reported rather than counted as connected. Which kinds are
/// checked (openings, doors, windows) is the rule's selection.
///
/// Each source holding a selected element or a host is also checked as a
/// whole: a source in which no host declares itself external is reported
/// against the source, since a building has an envelope.
pub struct OpeningSpaces;

/// A host's declared exposure and the facts behind it.
type Declaration = Result<(bool, Vec<Evidence>), Unavailable>;

struct Config<'a> {
    hosts: Traversal,
    host_selector: &'a Selector,
    external: PropertyRef<'a>,
    spaces: Traversal,
    space_selector: &'a Selector,
    /// Whether the spaces come from the derived adjacency, whose evidence
    /// records sides.
    sided: bool,
}

impl<'a> Config<'a> {
    fn parse(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let host_path = parameters
            .strings("host_path")?
            .ok_or_else(|| invalid("parameter `host_path` is required"))?;
        let space_path = parameters
            .strings("space_path")?
            .ok_or_else(|| invalid("parameter `space_path` is required"))?;
        let spaces = Traversal::path(space_path)?;
        let adjacency: Vec<bool> = spaces
            .steps()
            .iter()
            .map(|step| {
                step.relationships()
                    .iter()
                    .any(|r| is_adjacency(r.as_str()))
            })
            .collect();
        let sided = adjacency.contains(&true);
        if sided
            && (adjacency.len() != 1
                || spaces.steps().iter().any(|step| {
                    step.relationships().len() != 1
                        || step.direction() != TraversalDirection::Forward
                }))
        {
            return Err(invalid(
                "`axioval:derived.adjacent-space` must be the only `space_path` step, forward, \
                 so the sides it records are the checked element's",
            ));
        }
        Ok(Self {
            hosts: Traversal::path(host_path)?,
            host_selector: parameters.required_selector("host_selector")?,
            external: parameters.required_property("external_property")?,
            spaces,
            space_selector: parameters
                .selector("space_selector")?
                .unwrap_or(&Selector::All),
            sided,
        })
    }
}

/// Whether a relationship identity names the derived adjacency. A malformed
/// derived identity is left to the relationship service to refuse.
pub(crate) fn is_adjacency(relationship: &str) -> bool {
    SemanticRelationship::try_new(relationship)
        .ok()
        .and_then(|relationship| Derivation::parse(&relationship).ok().flatten())
        .is_some_and(|derivation| matches!(derivation, Derivation::AdjacentSpace { .. }))
}

impl RuleCapability for OpeningSpaces {
    fn id(&self) -> &'static str {
        "axioval:capability.opening-spaces"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("host_path", ParameterType::StringList),
            ParameterDescriptor::required("host_selector", ParameterType::Selector),
            ParameterDescriptor::required("external_property", ParameterType::PropertyReference),
            ParameterDescriptor::required("space_path", ParameterType::StringList),
            ParameterDescriptor::optional("space_selector", ParameterType::Selector),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match Config::parse(rule) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("opening-spaces: {message}"),
                );
            }
        };
        let hosts = Population::of(context, config.host_selector);
        let spaces = Population::of(context, config.space_selector);
        let (elements, mut evaluation) = select_objects(context, &rule.selector);
        let mut judge = Judge {
            context,
            rule,
            config: &config,
            hosts: &hosts,
            spaces: &spaces,
            declarations: BTreeMap::new(),
        };
        // Every source with an element to check or a wall to judge it by.
        let mut sources: BTreeSet<SourceId> = elements
            .iter()
            .map(|element| element.id.source.clone())
            .chain(
                evaluation
                    .not_evaluated_outcomes()
                    .iter()
                    .filter_map(|outcome| outcome.object_id())
                    .map(|object| object.source.clone()),
            )
            .collect();
        sources.extend(hosts.matched.iter().map(|host| host.source.clone()));
        for element in elements {
            match judge.element(element) {
                Ok(Some(finding)) => evaluation.push_finding(finding),
                Ok(None) => {}
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(element.id.clone(), reason, message);
                }
            }
        }
        for source in sources {
            judge.source(source, &mut evaluation);
        }
        evaluation
    }
}

struct Judge<'r, 'c> {
    context: &'r RuleContext<'c>,
    rule: &'r CompiledRule,
    config: &'r Config<'r>,
    hosts: &'r Population,
    spaces: &'r Population,
    /// Each host's declaration, resolved once per evaluation.
    declarations: BTreeMap<ObjectId, Declaration>,
}

impl Judge<'_, '_> {
    fn declaration(&mut self, host: &ObjectId) -> Declaration {
        if let Some(known) = self.declarations.get(host) {
            return known.clone();
        }
        let external = self.config.external;
        let declared = match self.context.project.object(host) {
            Some(object) => match resolve(self.context, object, external)? {
                Resolved::Present(property) => match &property.value {
                    PropertyValue::Boolean(value) => Ok((
                        *value,
                        property.evidence.iter().cloned().collect::<Vec<_>>(),
                    )),
                    PropertyValue::Null => Err((
                        NotEvaluatedReason::IncompleteEvidence,
                        format!("host {host} states no value for `{external}`"),
                    )),
                    _ => Err((
                        NotEvaluatedReason::InvalidEvidence,
                        format!("`{external}` of host {host} is not a boolean"),
                    )),
                },
                Resolved::Absent(_) => Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("host {host} does not declare `{external}`"),
                )),
            },
            None => Err(invalid(format!("host {host} is not in the project"))),
        };
        self.declarations.insert(host.clone(), declared.clone());
        declared
    }

    /// The objects of `population`, the universe a traversal may reach.
    fn universe(&self, population: &Population) -> Vec<&Object> {
        self.context
            .project
            .objects()
            .filter(|object| population.contains(&object.id))
            .collect()
    }

    /// The element's host walls, whether they are external, and the facts
    /// behind both.
    fn host(
        &mut self,
        element: &Object,
    ) -> Result<(Vec<ObjectId>, bool, Vec<Evidence>), Unavailable> {
        let config = self.config;
        let (reached, mut evidence) =
            config
                .hosts
                .related(self.context, &element.id, &self.universe(self.hosts))?;
        if let Some(undecided) = reached.iter().find(|id| !self.hosts.matched.contains(*id)) {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("whether {undecided} is a host wall is undecided"),
            ));
        }
        if reached.is_empty() {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "no host wall is reached via {}, so the spaces it needs are unknown",
                    config.hosts.relationship
                ),
            ));
        }
        let mut exposure = BTreeSet::new();
        for host in &reached {
            let (external, cited) = self.declaration(host)?;
            exposure.insert(external);
            evidence.extend(cited);
        }
        if exposure.len() != 1 {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("its host walls disagree on `{}`", config.external),
            ));
        }
        Ok((reached, exposure.contains(&true), evidence))
    }

    /// The finding for one element, `None` when it relates as required.
    fn element(&mut self, element: &Object) -> Result<Option<Finding>, Unavailable> {
        let config = self.config;
        let (hosts, external, mut evidence) = self.host(element)?;
        let host_text = hosts
            .iter()
            .map(|host| host.local_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let (wall, expected, requirement) = if external {
            ("an external wall", 1, "one space, the other side outside")
        } else {
            ("an internal wall", 2, "two spaces, one on each side")
        };

        let (related, cited) =
            config
                .spaces
                .related(self.context, &element.id, &self.universe(self.spaces))?;
        let decided: Vec<ObjectId> = related
            .iter()
            .filter(|id| self.spaces.matched.contains(*id))
            .cloned()
            .collect();
        let undecided = related.len() - decided.len();
        let count = decided.len();
        let via = &config.spaces.relationship;
        let violation = if count > expected || count + undecided < expected {
            Some(format!("relates to {count} space(s) via {via}"))
        } else if undecided > 0 {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "relates to {count} space(s) via {via} and {undecided} more that may be spaces"
                ),
            ));
        } else if config.sided {
            sides(&element.id, &decided, &cited, external)?
        } else {
            None
        };
        Ok(violation.map(|message| {
            evidence.extend(cited);
            let mut involved = hosts.clone();
            involved.extend(decided);
            finding(
                self.rule,
                &element.id,
                format!("{message}; in {wall} ({host_text}) it needs {requirement}"),
                evidence,
                involved,
            )
        }))
    }

    /// Reports a source in which no host is declared external.
    fn source(&mut self, source: SourceId, evaluation: &mut CapabilityEvaluation) {
        let walls: Vec<ObjectId> = self
            .hosts
            .matched
            .iter()
            .filter(|host| host.source == source)
            .cloned()
            .collect();
        let mut evidence = Vec::new();
        let mut unknown = Vec::new();
        for wall in &walls {
            match self.declaration(wall) {
                Ok((true, _)) => return,
                Ok((false, cited)) => evidence.extend(cited),
                Err(_) => unknown.push(wall.clone()),
            }
        }
        let undecided = self
            .hosts
            .undecided
            .iter()
            .filter(|host| host.source == source)
            .count();
        if !unknown.is_empty() || undecided > 0 {
            evaluation.push_source_not_evaluated(
                source.clone(),
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "opening-spaces: no wall in source `{source}` is declared external, but {} \
                     wall(s) do not declare `{}` and {undecided} more may be walls",
                    unknown.len(),
                    self.config.external
                ),
            );
            return;
        }
        evidence.sort_by(|a, b| (&a.source, &a.locator).cmp(&(&b.source, &b.locator)));
        evidence.dedup();
        let message = if walls.is_empty() {
            format!("source `{source}` has no host wall, so none is external")
        } else {
            format!(
                "none of the {} host wall(s) in source `{source}` is declared external",
                walls.len()
            )
        };
        evaluation.push_finding(
            Finding::new(
                self.rule.id.clone(),
                Scope::Source(source),
                severity(self.rule),
                message,
            )
            .with_evidence(evidence)
            .with_related(walls),
        );
    }
}

/// Why the derived adjacency's recorded faces fail the host's requirement,
/// `None` when they meet it: an internal host needs one space on each face,
/// an external one its space on one face and the other face outside.
fn sides(
    element: &ObjectId,
    spaces: &[ObjectId],
    cited: &[Evidence],
    external: bool,
) -> Result<Option<String>, Unavailable> {
    let mut sides: BTreeMap<&ObjectId, BTreeSet<AdjacentSide>> = BTreeMap::new();
    for space in spaces {
        let found: BTreeSet<AdjacentSide> = cited
            .iter()
            .filter_map(|item| adjacent_side(&item.locator, element, Some(space)))
            .collect();
        if found.is_empty() {
            return Err((
                NotEvaluatedReason::InvalidEvidence,
                format!("the adjacency evidence records no side for space {space}"),
            ));
        }
        sides.insert(space, found);
    }
    let placed = sides
        .iter()
        .map(|(space, faces)| {
            let faces: Vec<String> = faces.iter().map(ToString::to_string).collect();
            format!("{} on side {}", space.local_id, faces.join(" and "))
        })
        .collect::<Vec<_>>()
        .join(", ");
    let faces: Vec<&BTreeSet<AdjacentSide>> = sides.values().collect();
    if !external {
        let opposite = faces.iter().all(|face| face.len() == 1) && faces[0] != faces[1];
        return Ok((!opposite).then(|| format!("its spaces are not on opposite sides ({placed})")));
    }
    let face: Vec<AdjacentSide> = faces[0].iter().copied().collect();
    let [face] = face[..] else {
        return Ok(Some(format!("its one space lies on both sides ({placed})")));
    };
    let outside = cited
        .iter()
        .any(|item| adjacent_side(&item.locator, element, None) == Some(face.opposite()));
    if !outside {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("{placed}; the other side enters an object outside the space selection"),
        ));
    }
    Ok(None)
}
