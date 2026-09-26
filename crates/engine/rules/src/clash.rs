//! Source-neutral clash and clearance capability.
//!
//! ADR 0004: proximity is measured by a [`axioval_engine::ProximityServiceHandle`];
//! whether a measured overlap is a clash is decided here, against declared
//! tolerances.
//!
//! - A **hard clash** is one body reaching into another deeper than the
//!   declared penetration tolerance, or lying wholly inside it. Zero
//!   separation alone is not a clash: a slab resting on a wall has its
//!   surfaces meeting and nothing interpenetrating.
//! - A **clearance clash** is two bodies coming closer than the declared
//!   clearance without a hard clash.
//!
//! When neither body is a closed solid there is no inside to measure, so
//! surfaces that meet cannot be classified: the pair is reported not
//! evaluated rather than passed. Measurements on tessellated geometry are reported, and marked
//! approximate in both the message and the evidence.

use axioval_engine::{
    BodyContainment, CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, ProximityRequest, RuleCapability, RuleContext,
};
use axioval_ir::Finding;

use crate::pairs::{fidelity_note, length, prepare, reason, severity};

/// Reports bodies that interpenetrate, or come closer than a clearance.
pub struct Clash;

struct Declaration {
    penetration_tolerance: f64,
    clearance: Option<f64>,
}

fn declaration(rule: &CompiledRule) -> Option<Declaration> {
    let penetration_tolerance = length(rule, "penetration_tolerance_metres").ok()??;
    // A zero clearance is no clearance requirement; say so by omitting it.
    let clearance = length(rule, "clearance_metres").ok()?;
    if clearance == Some(0.0) {
        return None;
    }
    Some(Declaration {
        penetration_tolerance,
        clearance,
    })
}

impl RuleCapability for Clash {
    fn id(&self) -> &'static str {
        "axioval:capability.clash"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("counterparts", ParameterType::Selector),
            ParameterDescriptor::required("penetration_tolerance_metres", ParameterType::Number),
            ParameterDescriptor::optional("clearance_metres", ParameterType::Number),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = declaration(rule);
        let margin = declared
            .as_ref()
            .map(|declared| declared.clearance.unwrap_or(0.0));
        let prepared = match prepare(context, rule, margin) {
            Ok(prepared) => prepared,
            Err(refused) => return refused,
        };
        let Some(declared) = declared else {
            unreachable!("prepare refuses a missing margin");
        };
        let mut evaluation = CapabilityEvaluation::default();
        let mut unevaluated = prepared.unevaluated;

        for pair in &prepared.pairs {
            let (subject, counterpart) = (pair.subject(), pair.counterpart());
            let measured = ProximityRequest::try_new(subject.clone(), counterpart.clone())
                .and_then(|request| prepared.service.measure_proximity(&request))
                .and_then(|measured| {
                    // A measurement of another pair answers a different question.
                    if measured.request().subject() == subject
                        && measured.request().counterpart() == counterpart
                    {
                        Ok(measured)
                    } else {
                        Err(axioval_engine::ProximityError::InvalidMeasurement)
                    }
                });
            let measured = match measured {
                Ok(measured) => measured,
                Err(error) => {
                    unevaluated.push(
                        subject.clone(),
                        reason(error),
                        format!("proximity to {counterpart} could not be measured: {error}"),
                    );
                    continue;
                }
            };
            let note = fidelity_note(&measured);
            let hard = match (measured.containment(), measured.penetration_metres()) {
                (Some(BodyContainment::SubjectInsideCounterpart), _) => {
                    Some(format!("lies wholly inside {counterpart}{note}"))
                }
                (Some(BodyContainment::CounterpartInsideSubject), _) => {
                    Some(format!("wholly contains {counterpart}{note}"))
                }
                (None, Some(depth)) if depth > declared.penetration_tolerance => Some(format!(
                    "hard clash with {counterpart}: penetration {depth:.4} m exceeds tolerance {:.4} m{note}",
                    declared.penetration_tolerance
                )),
                (None, Some(_)) => None,
                (None, None) => {
                    if measured.separation_metres() == 0.0 {
                        unevaluated.push(
                            subject.clone(),
                            NotEvaluatedReason::IncompleteEvidence,
                            format!(
                                "surfaces meet {counterpart}, but neither body is a closed solid, so touching cannot be told from crossing"
                            ),
                        );
                        continue;
                    }
                    None
                }
            };
            let message = hard.or_else(|| {
                declared
                    .clearance
                    .filter(|clearance| measured.separation_metres() < *clearance)
                    .map(|clearance| {
                        format!(
                            "clearance clash with {counterpart}: separation {:.4} m below required {clearance:.4} m{note}",
                            measured.separation_metres()
                        )
                    })
            });
            if let Some(message) = message {
                evaluation.push_finding(
                    Finding {
                        rule_id: rule.id.clone(),
                        object_id: subject.clone(),
                        severity: severity(rule),
                        message,
                        related: Vec::new(),
                        evidence: vec![measured.evidence().clone()],
                    }
                    .with_related([counterpart.clone()]),
                );
            }
        }
        unevaluated.drain_into(&mut evaluation);
        evaluation
    }
}
