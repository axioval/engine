//! The `opening-spaces` implementation the template replaced, kept as the
//! template's parity reference.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::{Finding, Object, ObjectId, Scope, SourceId};

use super::{Config, Declaration, declaration, host, needs, sides, universe};
use crate::counts::Population;
use crate::opening_area::Picks;
use crate::pairs::severity;
use crate::selection::select_objects;
use crate::support::{Unavailable, finding};

/// Requires each selected door, window or opening to relate to the spaces
/// its host wall calls for, as [`crate::OpeningSpaces`] does.
pub struct OpeningSpaces;

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
        let declared = declaration(self.context, self.config.external, host);
        self.declarations.insert(host.clone(), declared.clone());
        declared
    }

    /// The finding for one element, `None` when it relates as required.
    fn element(&mut self, element: &Object) -> Result<Option<Finding>, Unavailable> {
        let (context, config, hosts) = (self.context, self.config, self.hosts);
        let (hosts, external, mut evidence) = host(
            context,
            (&config.hosts, Picks::of(hosts)),
            config.external,
            &element.id,
            &mut |host| self.declaration(host),
        )?;
        let host_text = hosts
            .iter()
            .map(|host| host.local_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let (wall, expected, requirement) = needs(external);

        let (related, cited) = config.spaces.related(
            self.context,
            &element.id,
            &universe(self.context, Picks::of(self.spaces)),
        )?;
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
