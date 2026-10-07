//! `distance` as it was implemented before it became a template (#283),
//! kept only as the parity reference the template is held to in the tests
//! (`parity-reference` feature). It is no capability of any registry. It
//! prepares the pairs and judges each subject through the same
//! `Pairs::among` and `verdicts` the template's measured `distance_items`
//! reads.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, Deviation, NotEvaluatedReason, ParameterDescriptor,
    RuleCapability, RuleContext, VerticalExtentServiceHandle,
};
use axioval_ir::{Evidence, ObjectId};

use super::{
    Caches, Declaration, Judged, Kinds, Mode, Pairs, Scope, Verdict, declaration, verdicts,
};
use crate::pairs::{Unevaluated, counterpart_selector, refuse_all, refuse_declaration};
use crate::selection::select_objects;
use crate::support::finding;

/// Requires counterparts to keep a declared distance from each subject, as
/// `distance` judged it before it became a template.
pub struct Distance;

impl RuleCapability for Distance {
    fn id(&self) -> &'static str {
        super::template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::parameters()
    }

    fn grades_deviation(&self) -> bool {
        true
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        if crate::object_parameters::has_object_parameters(rule) {
            return crate::object_parameters::per_object(self, context, rule);
        }
        let declared = match declaration(rule) {
            Ok(declared) => declared,
            Err((_, message)) => return refuse_declaration(context, rule, &message),
        };
        if declared.elevation.is_some()
            && context
                .services
                .get::<VerticalExtentServiceHandle>()
                .is_none()
        {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::MissingService,
                "distance: `elevation_overlap` needs the vertical-extent service, which is not \
                 registered",
            );
        }
        let (subjects, selection) = select_objects(context, &rule.selector);
        let Some(selector) = counterpart_selector(rule) else {
            return refuse_all(
                &subjects,
                selection,
                &NotEvaluatedReason::InvalidDeclaration,
                if declared.swings.0 || declared.swings.1 {
                    "`counterparts` is not a selector"
                } else {
                    "pairwise declaration is missing or not physically realisable"
                },
            );
        };
        let (counterparts, counterpart_selection) = select_objects(context, selector);
        let mut unevaluated = Unevaluated::default();
        for outcome in selection
            .not_evaluated_outcomes()
            .iter()
            .chain(counterpart_selection.not_evaluated_outcomes())
        {
            if let Some(object) = outcome.object_id() {
                unevaluated.push(
                    object.clone(),
                    outcome.reason().clone(),
                    outcome.message().to_owned(),
                );
            }
        }
        let pairs = match Pairs::among(context, &declared, (&subjects, &counterparts), unevaluated)
        {
            Ok(pairs) => pairs,
            Err((reason, message)) => return refuse_all(&subjects, selection, &reason, &message),
        };
        let kinds = declared
            .containers
            .as_ref()
            .map(|selector| Kinds::of(context, selector));
        let everything: Vec<ObjectId> = context
            .project
            .objects()
            .map(|object| object.id.clone())
            .collect();
        let mut caches = Caches::default();
        let mut evaluation = CapabilityEvaluation::default();
        for subject in pairs.subjects() {
            let mut scope = Scope::new(
                &declared,
                kinds.as_ref(),
                (context, &everything),
                &mut caches,
            );
            match verdicts((context, &declared), &pairs, &mut scope, subject) {
                Ok(judged) => judge(
                    &mut evaluation,
                    rule,
                    &declared,
                    subject,
                    (judged.apart, judged.within),
                    judged.scope_evidence,
                ),
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(subject.clone(), reason, message);
                }
            }
        }
        pairs.into_unevaluated().drain_into(&mut evaluation);
        evaluation
    }
}

/// Judges one subject's verdicts and records the outcome.
fn judge(
    evaluation: &mut CapabilityEvaluation,
    rule: &CompiledRule,
    declared: &Declaration,
    subject: &ObjectId,
    (apart, reach): (Option<Verdict>, Option<Verdict>),
    scope_evidence: Vec<Evidence>,
) {
    let mut messages = Vec::new();
    let mut related = Vec::new();
    let mut evidence = Vec::new();
    let mut open = None;
    // A finding missing both bounds grades by the worse; one naming
    // no distance against its bound is not graded.
    let mut deviations = Vec::new();
    for (verdict, keeping_apart) in [(apart, true), (reach, false)]
        .into_iter()
        .filter_map(|(verdict, apart)| verdict.map(|verdict| (verdict, apart)))
    {
        match verdict {
            Verdict::Finding {
                message,
                related: named,
                evidence: cited,
                judged,
            } => {
                messages.push(message);
                related.extend(named);
                evidence.extend(cited);
                deviations.push(match (judged, keeping_apart, declared.mode) {
                    (Judged::Distance(lower, upper), true, _) => declared
                        .minimum
                        .map(|minimum| Deviation::below(minimum, lower, upper)),
                    (Judged::Distance(lower, upper), false, Mode::Nearest) => declared
                        .maximum
                        .map(|maximum| Deviation::above(maximum, lower, upper)),
                    _ => None,
                });
            }
            Verdict::NotEvaluated(unavailable) => {
                open.get_or_insert(unavailable);
            }
            Verdict::Pass(_) => {}
        }
    }
    // A certain violation stands whatever is undecided.
    if !messages.is_empty() {
        evidence.extend(scope_evidence);
        let deviation = deviations
            .into_iter()
            .reduce(|a, b| a.zip(b).map(|(a, b)| a.worst(b)))
            .flatten();
        evaluation.push_finding_deviating(
            finding(rule, subject, messages.join("; "), evidence, related),
            deviation,
        );
    } else if let Some((reason, message)) = open {
        evaluation.push_object_not_evaluated(subject.clone(), reason, message);
    }
}
