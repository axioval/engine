//! Exact source-neutral external-wall validation capability.
//!
//! ADR 0004: envelope membership is measured by an
//! [`EnvelopeMembershipServiceHandle`]; whether the model's declaration agrees
//! with the derivation is decided here.
//!
//! Which objects bound the envelope is the ruleset's choice, not the host's,
//! and it travels in each [`EnvelopeMembershipRequest`] like the guard rule's
//! walking surfaces:
//!
//! - `all-spaces` derives around the objects `bounding_selector` selects;
//! - `gross-area-groups` derives around the members of the groups
//!   `gross_area_group_selector` selects, reached from each group along
//!   `gross_area_group_path`.
//!
//! `derivations` lists one or both, and each is reported on its own: every
//! finding and not-evaluated outcome names its derivation. With both, an
//! object on one envelope and not the other is reported too, whatever it
//! declares, and a source in which no selected object is declared external
//! is one finding against the source rather than one per object. An object a
//! selector cannot decide might be a bounding space, so that derivation is not
//! evaluated rather than derived around a guessed region.
//!
//! Applicability is deliberately absent. The source provider inspected a
//! model's industry domain and returned an "irrelevant" flag the rule had to
//! interpret; if a rule should not apply to a model, that belongs to the
//! selector, not behind the evidence seam.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, EnvelopeDerivation, EnvelopeMembershipError,
    EnvelopeMembershipEvidence, EnvelopeMembershipRequest, EnvelopeMembershipServiceHandle,
    NotEvaluatedReason, ParameterDescriptor, ParameterType, RuleCapability, RuleContext,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Finding, Object, ObjectId, Severity, SourceId};
use std::collections::{BTreeMap, BTreeSet};

use crate::counts::Population;
use crate::selection::select_objects;
use crate::support::{Parameters, Traversal, Unavailable, invalid};

/// The derivations the rule runs, each reported on its own.
const DERIVATIONS: &str = "derivations";
/// The objects the `all-spaces` envelope is derived around.
const BOUNDING_SELECTOR: &str = "bounding_selector";
/// The groups whose members bound the `gross-area-groups` envelope.
const GROUP_SELECTOR: &str = "gross_area_group_selector";
/// The relationship path from a gross-area group to its members.
const GROUP_PATH: &str = "gross_area_group_path";

/// Requires a model's declared external walls to match the derived envelope.
pub struct ExternalWallValidation;

impl RuleCapability for ExternalWallValidation {
    fn id(&self) -> &'static str {
        "axioval:capability.external-wall-validation"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required(DERIVATIONS, ParameterType::StringList),
            ParameterDescriptor::optional(BOUNDING_SELECTOR, ParameterType::Selector),
            ParameterDescriptor::optional(GROUP_SELECTOR, ParameterType::Selector),
            ParameterDescriptor::optional(GROUP_PATH, ParameterType::StringList),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        // Selection establishes that the rule applies at all and which
        // objects its findings are about; the derivation spans the model.
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        if selected.is_empty() {
            return evaluation;
        }

        let declaration = match Declaration::from_rule(rule) {
            Ok(declaration) => declaration,
            Err((reason, message)) => {
                evaluation
                    .push_not_evaluated(reason, format!("external-wall-validation: {message}"));
                return evaluation;
            }
        };

        let Some(service) = context.services.get::<EnvelopeMembershipServiceHandle>() else {
            evaluation.push_not_evaluated(
                NotEvaluatedReason::MissingService,
                "envelope-membership service is not registered",
            );
            return evaluation;
        };

        let selected: BTreeSet<&ObjectId> = selected.iter().map(|o| &o.id).collect();
        let mut measured = Vec::new();
        for derivation in &declaration.derivations {
            let name = derivation.as_str();
            let bounding = match declaration.bounding(context, *derivation) {
                Ok(bounding) => bounding,
                Err((reason, message)) => {
                    evaluation.push_not_evaluated(reason, format!("{name} envelope: {message}"));
                    continue;
                }
            };
            let request = EnvelopeMembershipRequest::new(*derivation, bounding);
            match service.measure_envelope_membership(&request) {
                Ok(evidence) => measured.push(evidence),
                Err(error) => evaluation.push_not_evaluated(
                    match error {
                        EnvelopeMembershipError::Unavailable
                        | EnvelopeMembershipError::UnsupportedDerivation => {
                            NotEvaluatedReason::IncompleteEvidence
                        }
                        EnvelopeMembershipError::InexactEvidence => {
                            NotEvaluatedReason::InvalidEvidence
                        }
                    },
                    format!("{name} envelope: {error}"),
                ),
            }
        }

        let undeclaring = judge_declarations(rule, &selected, &measured, &mut evaluation);
        for evidence in &measured {
            compare_declaration(rule, &selected, &undeclaring, evidence, &mut evaluation);
        }
        if let [first, second] = measured.as_slice() {
            compare_derivations(rule, &selected, first, second, &mut evaluation);
        }
        evaluation
    }
}

fn finding(
    rule: &CompiledRule,
    scope: axioval_ir::Scope,
    severity: Severity,
    message: String,
    evidence: Vec<Evidence>,
) -> Finding {
    Finding {
        id: None,
        decision: None,
        rule_id: rule.id.clone(),
        scope,
        severity,
        related: Vec::new(),
        message,
        evidence,
        location: None,
        categories: Vec::new(),
    }
}

/// The sources in which no selected object is declared external, each found
/// once as a whole rather than once per object on the envelope.
///
/// The declarations are the model's, the same in every derivation, so they
/// are read from every measurement together. A selected object whose
/// declaration is unstated, or whose body could not be measured, might be
/// the external one, so its source is not evaluated instead.
fn judge_declarations(
    rule: &CompiledRule,
    selected: &BTreeSet<&ObjectId>,
    measured: &[EnvelopeMembershipEvidence],
    evaluation: &mut CapabilityEvaluation,
) -> BTreeSet<SourceId> {
    let mut sources: BTreeMap<&SourceId, (bool, usize)> = BTreeMap::new();
    if measured.is_empty() {
        return BTreeSet::new();
    }
    for object in selected {
        sources.entry(&object.source).or_default();
    }
    for evidence in measured {
        for object in evidence.declared().iter().filter(|o| selected.contains(o)) {
            if let Some((declared, _)) = sources.get_mut(&object.source) {
                *declared = true;
            }
        }
    }
    let unknown: BTreeSet<&ObjectId> = measured
        .iter()
        .flat_map(EnvelopeMembershipEvidence::undeclared)
        .filter(|object| selected.contains(object))
        .collect();
    for object in unknown {
        if let Some((_, count)) = sources.get_mut(&object.source) {
            *count += 1;
        }
    }
    let evidence: Vec<Evidence> = measured
        .iter()
        .map(|measurement| measurement.evidence().clone())
        .collect();
    let mut undeclaring = BTreeSet::new();
    for (source, (declared, unknown)) in sources {
        if declared {
            continue;
        }
        if unknown > 0 {
            evaluation.push_source_not_evaluated(
                source.clone(),
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "no selected object is declared external, but {unknown} state neither external nor internal or could not be measured"
                ),
            );
            continue;
        }
        evaluation.push_finding(finding(
            rule,
            axioval_ir::Scope::Source(source.clone()),
            Severity::Error,
            "no selected object is declared external: the model declares no envelope".to_owned(),
            evidence.clone(),
        ));
        undeclaring.insert(source.clone());
    }
    undeclaring
}

/// Reports each selected object whose declaration disagrees with one
/// derivation against itself, so a reviewer opens the element rather than a
/// whole-model message. In a source declaring nothing external, the source
/// finding stands for every object on the envelope.
fn compare_declaration(
    rule: &CompiledRule,
    selected: &BTreeSet<&ObjectId>,
    undeclaring: &BTreeSet<SourceId>,
    measured: &EnvelopeMembershipEvidence,
    evaluation: &mut CapabilityEvaluation,
) {
    let name = measured.request().derivation().as_str();
    for object_id in measured.undeclared() {
        if selected.contains(object_id) {
            evaluation.push_object_not_evaluated(
                object_id.clone(),
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "not compared with the {name} envelope: the model states neither external nor internal, or its body could not be measured"
                ),
            );
        }
    }
    let declared_only = measured.declared_only().into_iter().map(|id| {
        (
            id,
            format!("declared external but not on the {name} envelope"),
        )
    });
    let derived_only = measured
        .derived_only()
        .into_iter()
        .filter(|id| !undeclaring.contains(&id.source))
        .map(|id| {
            (
                id,
                format!("on the {name} envelope but not declared external"),
            )
        });
    for (object_id, message) in declared_only.chain(derived_only) {
        if selected.contains(&object_id) {
            evaluation.push_finding(finding(
                rule,
                axioval_ir::Scope::Object(object_id),
                Severity::Warning,
                message,
                vec![measured.evidence().clone()],
            ));
        }
    }
}

/// Reports each selected object one derivation places on the envelope and
/// the other does not, whatever the model declares about it. An object
/// bounding either derivation is that envelope's inside and is not compared.
fn compare_derivations(
    rule: &CompiledRule,
    selected: &BTreeSet<&ObjectId>,
    first: &EnvelopeMembershipEvidence,
    second: &EnvelopeMembershipEvidence,
    evaluation: &mut CapabilityEvaluation,
) {
    let bounding: BTreeSet<&ObjectId> = first
        .request()
        .bounding()
        .iter()
        .chain(second.request().bounding())
        .collect();
    let on = |evidence: &EnvelopeMembershipEvidence| -> BTreeSet<ObjectId> {
        evidence.on_envelope().iter().cloned().collect()
    };
    let (on_first, on_second) = (on(first), on(second));
    for (only, on, off) in [
        (&on_first - &on_second, first, second),
        (&on_second - &on_first, second, first),
    ] {
        for object_id in only {
            if !selected.contains(&object_id) || bounding.contains(&object_id) {
                continue;
            }
            evaluation.push_finding(finding(
                rule,
                axioval_ir::Scope::Object(object_id),
                Severity::Warning,
                format!(
                    "on the {} envelope but not on the {} envelope",
                    on.request().derivation().as_str(),
                    off.request().derivation().as_str()
                ),
                vec![on.evidence().clone(), off.evidence().clone()],
            ));
        }
    }
}

/// The rule's derivations and the inputs that bound each.
struct Declaration<'rule> {
    derivations: Vec<EnvelopeDerivation>,
    bounding: Option<&'rule Selector>,
    groups: Option<(&'rule Selector, Traversal<'rule>)>,
}

impl<'rule> Declaration<'rule> {
    /// Reads and cross-checks the declaration before anything is measured, so
    /// a derivation without its bounding input is refused as a declaration
    /// error rather than measured around nothing.
    fn from_rule(rule: &'rule CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let listed = parameters
            .strings(DERIVATIONS)?
            .ok_or_else(|| invalid(format!("parameter `{DERIVATIONS}` is required")))?;
        if listed.is_empty() {
            return Err(invalid(format!("`{DERIVATIONS}` names no derivation")));
        }
        let mut derivations = Vec::new();
        for name in listed {
            let derivation = match name.as_str() {
                "all-spaces" => EnvelopeDerivation::AllSpaces,
                "gross-area-groups" => EnvelopeDerivation::GrossAreaGroups,
                other => {
                    return Err(invalid(format!(
                        "envelope derivation `{other}` must be 'all-spaces' or 'gross-area-groups'"
                    )));
                }
            };
            if derivations.contains(&derivation) {
                return Err(invalid(format!("`{DERIVATIONS}` lists `{name}` twice")));
            }
            derivations.push(derivation);
        }

        let bounding = parameters.selector(BOUNDING_SELECTOR)?;
        let groups = match (
            parameters.selector(GROUP_SELECTOR)?,
            parameters.strings(GROUP_PATH)?,
        ) {
            (Some(selector), Some(path)) => Some((selector, Traversal::path(path)?)),
            (None, None) => None,
            _ => {
                return Err(invalid(format!(
                    "declare `{GROUP_SELECTOR}` and `{GROUP_PATH}` together"
                )));
            }
        };
        for derivation in &derivations {
            match derivation {
                EnvelopeDerivation::AllSpaces if bounding.is_none() => {
                    return Err(invalid(format!(
                        "the all-spaces derivation needs `{BOUNDING_SELECTOR}`"
                    )));
                }
                EnvelopeDerivation::GrossAreaGroups if groups.is_none() => {
                    return Err(invalid(format!(
                        "the gross-area-groups derivation needs `{GROUP_SELECTOR}` and `{GROUP_PATH}`"
                    )));
                }
                _ => {}
            }
        }
        Ok(Self {
            derivations,
            bounding,
            groups,
        })
    }

    /// The objects `derivation` is derived around, resolved exactly.
    fn bounding(
        &self,
        context: &RuleContext<'_>,
        derivation: EnvelopeDerivation,
    ) -> Result<Vec<ObjectId>, Unavailable> {
        match derivation {
            EnvelopeDerivation::GrossAreaGroups => {
                let (selector, path) = self
                    .groups
                    .as_ref()
                    .ok_or_else(|| invalid("gross-area groups are not declared"))?;
                let groups = decided(context, GROUP_SELECTOR, selector)?;
                // A group's members may be of any kind; the path, not this
                // rule, says what belongs to it.
                let universe: Vec<&Object> = context.project.objects().collect();
                let mut members = BTreeSet::new();
                for group in &groups {
                    let (reached, _) = path.related(context, group, &universe)?;
                    members.extend(reached);
                }
                if members.is_empty() {
                    return Err((
                        NotEvaluatedReason::IncompleteEvidence,
                        format!(
                            "the {} selected gross-area group(s) have no member along `{GROUP_PATH}`",
                            groups.len()
                        ),
                    ));
                }
                Ok(members.into_iter().collect())
            }
            EnvelopeDerivation::AllSpaces => {
                let selector = self
                    .bounding
                    .ok_or_else(|| invalid(format!("`{BOUNDING_SELECTOR}` is not declared")))?;
                decided(context, BOUNDING_SELECTOR, selector)
            }
            _ => Err(invalid("the derivation is not supported")),
        }
    }
}

/// The objects `selector` picks, when it decides every object and picks one.
///
/// An undecided object might bound the envelope; deriving without it or with
/// it could invent or hide a discrepancy, so the derivation is not evaluated.
/// Nothing selected leaves no region to derive around.
fn decided(
    context: &RuleContext<'_>,
    name: &str,
    selector: &Selector,
) -> Result<Vec<ObjectId>, Unavailable> {
    let population = Population::of(context, selector);
    if !population.undecided.is_empty() {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "`{name}` cannot be decided for {} object(s)",
                population.undecided.len()
            ),
        ));
    }
    if population.matched.is_empty() {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("`{name}` selects no object"),
        ));
    }
    Ok(population.matched.into_iter().collect())
}
