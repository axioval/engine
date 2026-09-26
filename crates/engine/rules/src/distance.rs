//! Source-neutral distance capability.
//!
//! Each subject's distance to its nearest counterpart must lie within declared
//! bounds: at least a minimum (keep apart), at most a maximum (stay within
//! reach), or both. Distance is surface to surface, zero for bodies that meet.
//!
//! A maximum is the asymmetric case. Finding one counterpart close enough
//! proves compliance; concluding that none is close enough needs every
//! counterpart measured. The broad phase is complete, so counterparts it does
//! not propose are proven beyond the maximum, but a counterpart whose extent
//! or distance could not be read might be the near one. The subject is then
//! not evaluated rather than reported. A minimum is symmetric in the same way:
//! anything unmeasured might be the counterpart that comes too close.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    ProximityError, ProximityEvidence, ProximityRequest, RuleCapability, RuleContext,
};
use axioval_ir::{Finding, ObjectId};

use crate::pairs::{Prepared, fidelity_note, length, prepare, reason, severity};

/// Requires the nearest counterpart to lie within a distance range.
pub struct Distance;

#[derive(Clone, Copy)]
struct Range {
    minimum: Option<f64>,
    maximum: Option<f64>,
}

fn declaration(rule: &CompiledRule) -> Option<Range> {
    let minimum = length(rule, "minimum_metres").ok()?;
    let maximum = length(rule, "maximum_metres").ok()?;
    match (minimum, maximum) {
        (None, None) => None,
        (Some(minimum), Some(maximum)) if minimum > maximum => None,
        (minimum, maximum) => Some(Range { minimum, maximum }),
    }
}

/// What one subject's candidates measured.
struct Measured {
    nearest: Option<ProximityEvidence>,
    failures: Vec<(ObjectId, ProximityError)>,
}

/// Measures `subject` against every counterpart the broad phase proposed.
fn measure(prepared: &Prepared<'_>, subject: &ObjectId) -> Measured {
    // The broad phase reports each pair once, in either orientation.
    let candidates = prepared.pairs.iter().filter_map(|pair| {
        if pair.subject() == subject {
            Some(pair.counterpart())
        } else if pair.counterpart() == subject && prepared.counterparts.contains(pair.subject()) {
            Some(pair.subject())
        } else {
            None
        }
    });
    let mut measured = Measured {
        nearest: None,
        failures: Vec::new(),
    };
    for counterpart in candidates {
        let outcome = ProximityRequest::try_new(subject.clone(), counterpart.clone())
            .and_then(|request| prepared.service.measure_proximity(&request))
            .and_then(|evidence| {
                // A measurement of another pair answers a different question.
                if evidence.request().subject() == subject
                    && evidence.request().counterpart() == counterpart
                {
                    Ok(evidence)
                } else {
                    Err(ProximityError::InvalidMeasurement)
                }
            });
        match outcome {
            Ok(evidence) => {
                if measured
                    .nearest
                    .as_ref()
                    .is_none_or(|best| evidence.separation_metres() < best.separation_metres())
                {
                    measured.nearest = Some(evidence);
                }
            }
            Err(error) => measured.failures.push((counterpart.clone(), error)),
        }
    }
    measured
}

/// The verdict for one subject: a finding message, a refusal, or a pass.
enum Verdict {
    Finding(String),
    NotEvaluated(NotEvaluatedReason, String),
    Pass,
}

fn judge(range: Range, measured: &Measured, unknown_counterpart: bool) -> Verdict {
    let separation = measured
        .nearest
        .as_ref()
        .map(ProximityEvidence::separation_metres);
    if let (Some(minimum), Some(nearest)) = (range.minimum, &measured.nearest) {
        let distance = nearest.separation_metres();
        if distance < minimum {
            return Verdict::Finding(format!(
                "nearest counterpart {} is {distance:.4} m away, closer than the required {minimum:.4} m{}",
                nearest.request().counterpart(),
                fidelity_note(nearest)
            ));
        }
    }
    let too_far = range
        .maximum
        .filter(|maximum| separation.is_none_or(|distance| distance > *maximum));
    // A minimum could be broken, or an unmet maximum met, by what was not
    // measured. A maximum already met stays met.
    if (!measured.failures.is_empty() || unknown_counterpart)
        && (range.minimum.is_some() || too_far.is_some())
    {
        return match measured.failures.first() {
            Some((counterpart, error)) => Verdict::NotEvaluated(
                reason(*error),
                format!("distance to {counterpart} could not be measured: {error}"),
            ),
            None => Verdict::NotEvaluated(
                NotEvaluatedReason::IncompleteEvidence,
                "a counterpart's extent could not be read, so the nearest one is unknown"
                    .to_owned(),
            ),
        };
    }
    match (too_far, &measured.nearest) {
        (Some(maximum), Some(nearest)) => Verdict::Finding(format!(
            "nearest counterpart {} is {:.4} m away, farther than the allowed {maximum:.4} m{}",
            nearest.request().counterpart(),
            nearest.separation_metres(),
            fidelity_note(nearest)
        )),
        (Some(maximum), None) => {
            Verdict::Finding(format!("no counterpart lies within {maximum:.4} m"))
        }
        (None, _) => Verdict::Pass,
    }
}

impl RuleCapability for Distance {
    fn id(&self) -> &'static str {
        "axioval:capability.distance"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("counterparts", ParameterType::Selector),
            ParameterDescriptor::optional("minimum_metres", ParameterType::Number),
            ParameterDescriptor::optional("maximum_metres", ParameterType::Number),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let range = declaration(rule);
        // Search as far as the farther bound: the nearest counterpart may lie
        // anywhere inside a maximum.
        let margin = range.map(|range| range.maximum.or(range.minimum).unwrap_or(0.0));
        let prepared = match prepare(context, rule, margin) {
            Ok(prepared) => prepared,
            Err(refused) => return refused,
        };
        let Some(range) = range else {
            unreachable!("prepare refuses a missing margin");
        };
        let mut evaluation = CapabilityEvaluation::default();

        for subject in &prepared.subjects {
            let measured = measure(&prepared, subject);
            let unknown_counterpart = prepared
                .unmeasurable_counterparts
                .iter()
                .any(|counterpart| counterpart != subject);
            match judge(range, &measured, unknown_counterpart) {
                Verdict::Pass => {}
                Verdict::NotEvaluated(reason, message) => {
                    evaluation.push_object_not_evaluated(subject.clone(), reason, message);
                }
                Verdict::Finding(message) => evaluation.push_finding(
                    Finding {
                        rule_id: rule.id.clone(),
                        object_id: subject.clone(),
                        severity: severity(rule),
                        message,
                        related: Vec::new(),
                        evidence: measured
                            .nearest
                            .iter()
                            .map(|nearest| nearest.evidence().clone())
                            .collect(),
                    }
                    .with_related(
                        measured
                            .nearest
                            .iter()
                            .map(|nearest| nearest.request().counterpart().clone()),
                    ),
                ),
            }
        }
        prepared.unevaluated.drain_into(&mut evaluation);
        evaluation
    }
}
