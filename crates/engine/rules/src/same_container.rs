//! `same-container`: an object lies in the same containers as the objects a
//! path reaches from it, such as a door on its host wall's storey.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::{Evidence, Finding, Object, ObjectId};

use crate::counts::{Population, tally};
use crate::selection::select_objects;
use crate::support::{Parameters, Traversal, Unavailable, finding, invalid};

/// Requires each selected object to share its nearest containers with every
/// counterpart `counterpart_path` reaches from it.
///
/// A door or window must stand on its host wall's storey: from the door,
/// `counterpart_path` `IfcRelFillsElement:backward` then
/// `IfcRelVoidsElement:backward` reaches the host wall, and the storeys
/// (`container_selector`) are climbed to along
/// `IfcRelContainedInSpatialStructure` backwards, as `property-comparison`'s
/// container modes climb: the declared steps in any order, any number of
/// times, stopping at each container reached. The object and each
/// counterpart must reach the same set of containers; one in none while the
/// other is in one differs too.
///
/// `counterpart_selector` restricts which reached objects count (every
/// object by default). An object reaching no counterpart has nothing to
/// agree with and passes. A container selector that cannot decide an object
/// leaves every selected object not evaluated, since it may be the
/// container; an undecided counterpart leaves the object not evaluated
/// unless a decided one already differs; a refused relationship answer
/// leaves it not evaluated.
pub struct SameContainer;

struct Config<'a> {
    counterparts: Traversal<'a>,
    counterpart_selector: Option<&'a axioval_ir::contract::Selector>,
    containers: &'a axioval_ir::contract::Selector,
    climb: Traversal<'a>,
}

impl<'a> Config<'a> {
    fn parse(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let path = parameters
            .strings("counterpart_path")?
            .ok_or_else(|| invalid("parameter `counterpart_path` is required"))?;
        let climb = parameters
            .traversal()?
            .ok_or_else(|| invalid("containers are climbed to along `relationship` or `path`"))?;
        if climb.follows_chain() {
            return Err(invalid(
                "containers are climbed transitively; `follow_chain` does not apply",
            ));
        }
        Ok(Self {
            counterparts: Traversal::path(path)?,
            counterpart_selector: parameters.selector("counterpart_selector")?,
            containers: parameters.required_selector("container_selector")?,
            climb,
        })
    }
}

impl RuleCapability for SameContainer {
    fn id(&self) -> &'static str {
        "axioval:capability.same-container"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("counterpart_path", ParameterType::StringList),
            ParameterDescriptor::optional("counterpart_selector", ParameterType::Selector),
            ParameterDescriptor::required("container_selector", ParameterType::Selector),
        ]
        .into_iter()
        .chain(crate::support::traversal_parameters())
        .collect()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match Config::parse(rule) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("same-container: {message}"),
                );
            }
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let (containers, outcomes) = select_objects(context, config.containers);
        if !outcomes.not_evaluated_outcomes().is_empty() {
            // An undecided object may be the container two objects share.
            for object in selected {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    "container selector was not evaluated conclusively",
                );
            }
            return evaluation;
        }
        let containers: BTreeSet<ObjectId> = containers
            .into_iter()
            .map(|object| object.id.clone())
            .collect();
        let population = Population::of(
            context,
            config
                .counterpart_selector
                .unwrap_or(&axioval_ir::contract::Selector::All),
        );
        let mut judge = Judge {
            context,
            rule,
            config: &config,
            containers: &containers,
            population: &population,
            climbed: BTreeMap::new(),
        };
        for object in selected {
            match judge.object(object) {
                Ok(Some(found)) => evaluation.push_finding(found),
                Ok(None) => {}
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}

/// Nearest containers and the evidence of the climb, or why it failed.
type Climb = Result<(BTreeSet<ObjectId>, Vec<Evidence>), Unavailable>;

struct Judge<'c, 'a> {
    context: &'c RuleContext<'a>,
    rule: &'c CompiledRule,
    config: &'c Config<'c>,
    containers: &'c BTreeSet<ObjectId>,
    population: &'c Population,
    /// Each object is climbed from once per evaluation.
    climbed: BTreeMap<ObjectId, Climb>,
}

impl Judge<'_, '_> {
    fn nearest(&mut self, object: &ObjectId) -> Climb {
        let (context, climb, containers) = (self.context, &self.config.climb, self.containers);
        self.climbed
            .entry(object.clone())
            .or_insert_with(|| climb.nearest_containers(context, object, containers))
            .clone()
    }

    /// A finding when a decided counterpart lies elsewhere; nothing when
    /// every counterpart agrees or there is none.
    fn object(&mut self, object: &Object) -> Result<Option<Finding>, Unavailable> {
        let via = &self.config.counterparts.relationship;
        let reached = tally(
            self.context,
            Some(&self.config.counterparts),
            object,
            self.population,
        )?;
        let undecided = || {
            (
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{} counterpart(s) via {via} cannot be decided and may lie elsewhere",
                    reached.undecided
                ),
            )
        };
        if reached.decided.is_empty() {
            return if reached.undecided == 0 {
                Ok(None)
            } else {
                Err(undecided())
            };
        }
        let (mine, mut evidence) = self.nearest(&object.id)?;
        evidence.extend(reached.evidence.iter().cloned());
        let mut differing = Vec::new();
        for counterpart in &reached.decided {
            let (theirs, cited) = self.nearest(counterpart)?;
            if theirs != mine {
                evidence.extend(cited);
                differing.push((counterpart.clone(), theirs));
            }
        }
        if differing.is_empty() {
            return if reached.undecided == 0 {
                Ok(None)
            } else {
                Err(undecided())
            };
        }
        let listed = differing
            .iter()
            .map(|(counterpart, theirs)| {
                format!("{} is in {}", counterpart.local_id, names(theirs))
            })
            .collect::<Vec<_>>()
            .join("; ");
        let mut related: Vec<ObjectId> = mine.iter().cloned().collect();
        for (counterpart, theirs) in differing {
            related.push(counterpart);
            related.extend(theirs);
        }
        related.sort();
        related.dedup();
        Ok(Some(finding(
            self.rule,
            &object.id,
            format!(
                "in {}, but its counterpart via {via} is not: {listed}",
                names(&mine)
            ),
            evidence,
            related,
        )))
    }
}

/// Containers as a reviewer reads them: `no container`, or their local ids.
fn names(containers: &BTreeSet<ObjectId>) -> String {
    if containers.is_empty() {
        "no container".to_owned()
    } else {
        containers
            .iter()
            .map(|container| container.local_id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}
