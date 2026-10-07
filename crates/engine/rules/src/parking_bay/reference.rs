//! `parking-bay` as it was implemented before it became a template
//! (#283), kept only as the parity reference the template is held to in
//! the tests (`parity-reference` feature). It is no capability of any
//! registry. It measures and searches through the same `Bay::steps` the
//! template's measured `parking_bay` reads.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, RuleCapability,
    RuleContext,
};
use axioval_ir::contract::Selector;

use super::{
    Bay, Check, Filtering, Matter, NAME, Nearby, Reference, Services, bounded, judge_count, parse,
};
use crate::orientation::Tri;
use crate::pairs::refuse_all;
use crate::selection::select_objects;
use crate::support::{Parameters, finding};

/// Requires each selected parking bay to have its size, orientation and
/// obstructions allowed, as `parking-bay` judged it before it became a
/// template.
pub struct ParkingBay;

impl RuleCapability for ParkingBay {
    fn id(&self) -> &'static str {
        super::template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::parameters()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match parse(&Parameters(rule)) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(reason, format!("{NAME}: {message}"));
            }
        };
        let (bays, evaluation) = select_objects(context, &rule.selector);
        let services = match Services::of(context, &config) {
            Ok(services) => services,
            Err((reason, message)) => return refuse_all(&bays, evaluation, &reason, &message),
        };
        let near = |selector: &Selector, reach: f64| {
            Nearby::find(context, &services, selector, &bays, reach)
        };
        let references = match config.orientation.as_ref().map(|o| match &o.reference {
            Reference::Aisles { aisles, reach, .. } => near(aisles, *reach),
            Reference::Neighbours { reach } => near(&rule.selector, *reach),
        }) {
            Some(Err((reason, message))) => {
                return refuse_all(&bays, evaluation, &reason, &message);
            }
            other => other.map(Result::unwrap),
        };
        let obstacles = match config
            .obstructions
            .as_ref()
            .map(|o| near(o.obstacles, o.reach))
        {
            Some(Err((reason, message))) => {
                return refuse_all(&bays, evaluation, &reason, &message);
            }
            other => other.map(Result::unwrap),
        };
        let mut evaluation = evaluation;
        for bay in bays {
            let judged = Bay {
                config: &config,
                services: &services,
                object: bay,
            };
            let (steps, filtering) = judged.steps(references.as_ref(), obstacles.as_ref());
            for check in checks(steps, filtering.as_ref()) {
                match check {
                    Ok(None) => {}
                    Ok(Some((message, evidence, related))) => {
                        evaluation.push_finding(finding(rule, &bay.id, message, evidence, related));
                    }
                    Err((reason, message)) => {
                        evaluation.push_object_not_evaluated(bay.id.clone(), reason, message);
                    }
                }
            }
        }
        evaluation
    }
}

/// The checks of a bay's steps, a size's applied only as far as the
/// filters say it applies to the bay.
fn checks(steps: Vec<Matter>, filtering: Option<&Filtering>) -> Vec<Check> {
    let mut checks = Vec::new();
    for step in steps {
        let check = match step {
            Matter::Judged(check) => check,
            Matter::Count(counting) => judge_count(&counting),
            Matter::Size(sized) => {
                let check = match sized.measured {
                    Ok((what, measured, evidence)) => {
                        bounded(&what, measured, sized.bounds.0, sized.bounds.1, evidence)
                    }
                    Err(unavailable) => Err(unavailable),
                };
                match filtering {
                    None => check,
                    Some(filtering) => match filtering.applies {
                        Tri::No => continue,
                        Tri::Yes => check.map(|found| {
                            found.map(|(message, mut cited, related)| {
                                cited.extend(filtering.evidence.iter().cloned());
                                (
                                    format!("{message} (a bay with {})", filtering.states),
                                    cited,
                                    related,
                                )
                            })
                        }),
                        Tri::Maybe => match check {
                            Ok(None) => Ok(None),
                            Ok(Some((message, _, _))) => Err((
                                NotEvaluatedReason::IncompleteEvidence,
                                format!("{message}, if the bound applies to it: {}", filtering.why),
                            )),
                            Err((reason, message)) => {
                                Err((reason, format!("{message}; {}", filtering.why)))
                            }
                        },
                    },
                }
            }
        };
        checks.push(check);
    }
    checks
}
